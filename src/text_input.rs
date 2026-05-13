use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    app::App,
    types::{ActiveAction, CheckoutRemotePhase},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInputKeyResult {
    Ignored,
    Updated,
    Submit,
    Cancel,
    Complete,
}

pub fn is_active(app: &App) -> bool {
    match app.active_action {
        ActiveAction::NewBranch => !app.new_branch_loading,
        ActiveAction::Rename => !app.rename_loading,
        ActiveAction::SyncPr => !app.sync_pr_loading,
        ActiveAction::CloneRepo => !app.clone_loading,
        ActiveAction::CheckoutRemote => {
            !app.checkout_remote_is_loading()
                && matches!(
                    app.checkout_remote_phase,
                    CheckoutRemotePhase::SelectRemote | CheckoutRemotePhase::EnterBranch
                )
        }
        _ => false,
    }
}

pub fn wants_mouse_capture(app: &App) -> bool {
    app.active_action != ActiveAction::CloneRepo && !is_active(app)
}

pub fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> TextInputKeyResult {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);

    match code {
        KeyCode::Esc => TextInputKeyResult::Cancel,
        KeyCode::Enter => TextInputKeyResult::Submit,
        KeyCode::Backspace => {
            app.input_backspace();
            TextInputKeyResult::Updated
        }
        KeyCode::Delete if ctrl => {
            app.input_delete_next_word();
            TextInputKeyResult::Updated
        }
        KeyCode::Delete => {
            app.input_delete();
            TextInputKeyResult::Updated
        }
        KeyCode::Left if ctrl => {
            app.input_left_word();
            TextInputKeyResult::Updated
        }
        KeyCode::Left => {
            app.input_left();
            TextInputKeyResult::Updated
        }
        KeyCode::Right if ctrl => {
            app.input_right_word();
            TextInputKeyResult::Updated
        }
        KeyCode::Right => {
            app.input_right();
            TextInputKeyResult::Updated
        }
        KeyCode::Home => {
            app.input_home();
            TextInputKeyResult::Updated
        }
        KeyCode::End => {
            app.input_end();
            TextInputKeyResult::Updated
        }
        KeyCode::Tab => TextInputKeyResult::Complete,
        KeyCode::Char(c) if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
            app.input_char(c);
            TextInputKeyResult::Updated
        }
        _ => TextInputKeyResult::Ignored,
    }
}

pub fn handle_paste(app: &mut App, text: &str) -> bool {
    if !is_active(app) {
        return false;
    }

    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty() {
        return false;
    }

    app.input_str(text);
    true
}
