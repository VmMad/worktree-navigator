use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

pub struct PtySession {
    pub parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
}

impl PtySession {
    pub fn spawn(cwd: &str, cols: u16, rows: u16) -> Result<Self> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(cwd);
        // Pass a clean environment inheriting from parent
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let _child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn shell")?;

        let writer = pair
            .master
            .take_writer()
            .context("Failed to get PTY writer")?;

        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to get PTY reader")?;

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 1000)));

        // Background thread: read PTY output → feed vt100 parser in small chunks
        // to minimise the time the lock is held, letting the UI thread acquire it
        // between chunks.
        let parser_clone = Arc::clone(&parser);
        std::thread::spawn(move || {
            const CHUNK_SIZE: usize = 512;
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Only split into chunks when the read is large enough to
                        // warrant it; small reads acquire the lock once directly.
                        if n <= CHUNK_SIZE {
                            if let Ok(mut p) = parser_clone.lock() {
                                p.process(&buf[..n]);
                            }
                        } else {
                            for chunk in buf[..n].chunks(CHUNK_SIZE) {
                                if let Ok(mut p) = parser_clone.lock() {
                                    p.process(chunk);
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            parser,
            writer,
            _master: pair.master,
        })
    }

    pub fn write_input(&mut self, data: &[u8]) {
        let _ = self.writer.write_all(data);
        let _ = self.writer.flush();
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self._master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows, cols);
        }
    }
}
