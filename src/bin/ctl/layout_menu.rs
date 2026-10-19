//! Run the layout chooser in the client so waiting for a choice never blocks
//! either compositor backend.
use std::io::Write;
use std::process::{Command, Stdio};

use instantwm::actions::NamedAction;
use instantwm::ipc_types::{IpcCommand, Response, TagCommand};
use instantwm::layouts::LayoutCommand;

use super::ipc;

struct Choice {
    value: &'static str,
    label: String,
    icon: &'static str,
    color: &'static str,
    action: Option<NamedAction>,
}

impl Choice {
    fn line(&self) -> String {
        format!(
            "{{{} icon={} value={}}} {}",
            self.color, self.icon, self.value, self.label
        )
    }
}

fn choices(active: LayoutCommand) -> Vec<Choice> {
    let mut entries: Vec<_> = LayoutCommand::all()
        .iter()
        .map(|&layout| {
            let (label, icon) = match layout {
                LayoutCommand::Tile => ("Main window on left", "md-view-column"),
                LayoutCommand::Grid => ("Grid", "md-view-grid"),
                LayoutCommand::BottomStack => ("Main window on top", "md-dock-bottom"),
                LayoutCommand::Floating => ("Floating windows", "fa-window-restore"),
                LayoutCommand::Maximized => ("Maximized windows", "fa-window-maximize"),
            };
            let current = layout == active;
            Choice {
                value: layout.name(),
                label: format!("{label}{}", if current { " (current)" } else { "" }),
                icon,
                color: if current { "green" } else { "blue" },
                // Choosing the current layout must not silently reset edits.
                action: (!current).then_some(NamedAction::SetLayout(layout)),
            }
        })
        .collect();
    entries.push(Choice {
        value: "reset",
        label: "Reset window arrangement".into(),
        icon: "fa-arrow-rotate-left",
        color: "yellow",
        action: Some(NamedAction::ResetLayout),
    });
    entries
}

fn selected_action(entries: &[Choice], answer: &str) -> Option<NamedAction> {
    entries
        .iter()
        .find(|entry| entry.value == answer)
        .and_then(|entry| entry.action.clone())
}

fn menu_command(active: LayoutCommand) -> Command {
    let mut command = Command::new("instantmenu");
    command
        .args([
            "--follow-cursor",
            "--border-width",
            "4",
            "--width",
            "auto",
            "--line-height",
            "auto",
            "--lines",
            "6",
            "--placeholder",
            "Choose window layout",
            "--insensitive",
            "--reject-no-match",
            "--preselect",
            active.name(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    command
}

// A chooser can stay open while the user switches workspaces. Dismiss a
// stale choice rather than applying it to a different tag or monitor.
fn context(ignore_version: bool) -> Result<(u64, Vec<u32>), String> {
    let monitor =
        match ipc::send(IpcCommand::LayoutStatus, ignore_version).map_err(|e| e.to_string())? {
            Response::LayoutStatus(status) => status.monitor_id,
            response => return Err(format!("unexpected layout status: {response:?}")),
        };
    let tags = match ipc::send(IpcCommand::Tag(TagCommand::List), ignore_version)
        .map_err(|e| e.to_string())?
    {
        Response::TagList(tags) => tags
            .into_iter()
            .filter(|tag| tag.selected)
            .map(|tag| tag.index)
            .collect(),
        response => return Err(format!("unexpected tag list: {response:?}")),
    };
    Ok((monitor, tags))
}

pub fn run(ignore_version: bool) -> Result<(), String> {
    let initial_context = context(ignore_version)?;
    let active =
        match ipc::send(IpcCommand::LayoutStatus, ignore_version).map_err(|e| e.to_string())? {
            Response::LayoutStatus(status) => {
                LayoutCommand::from_name(&status.layout.name).ok_or("unknown active layout")?
            }
            Response::Err(message) => return Err(message),
            response => return Err(format!("unexpected layout status: {response:?}")),
        };
    let entries = choices(active);
    let mut child = menu_command(active)
        .spawn()
        .map_err(|e| format!("failed to open layout chooser: {e}"))?;
    let input = entries
        .iter()
        .map(Choice::line)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let write_result = child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes());
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    // Dismissal can close stdin early. It always leaves the arrangement alone.
    if !output.status.success() {
        return Ok(());
    }
    write_result.map_err(|e| e.to_string())?;
    let answer = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let Some(action) = selected_action(&entries, answer.trim_end()) else {
        return Ok(());
    };
    if context(ignore_version)? != initial_context {
        return Ok(());
    }
    match ipc::send(
        IpcCommand::RunAction {
            name: action.name().into(),
            args: action.args(),
        },
        ignore_version,
    )
    .map_err(|e| e.to_string())?
    {
        Response::Ok => Ok(()),
        Response::Err(message) => Err(message),
        response => Err(format!("unexpected layout response: {response:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooser_separates_switching_from_resetting() {
        for &active in LayoutCommand::all() {
            let entries = choices(active);
            assert_eq!(selected_action(&entries, active.name()), None);
            assert_eq!(
                selected_action(&entries, "reset"),
                Some(NamedAction::ResetLayout)
            );
            assert_eq!(selected_action(&entries, ""), None);
            assert_eq!(selected_action(&entries, "arbitrary typed text"), None);
            for &other in LayoutCommand::all() {
                if other != active {
                    assert_eq!(
                        selected_action(&entries, other.name()),
                        Some(NamedAction::SetLayout(other))
                    );
                }
            }
        }
    }

    #[test]
    fn decorated_labels_do_not_control_action_dispatch() {
        let mut entries = choices(LayoutCommand::Grid);
        for entry in &mut entries {
            entry.label = "A translated label".into();
            assert!(entry.line().contains(&format!("value={}", entry.value)));
            assert!(entry.line().contains("icon="));
        }
        assert_eq!(
            selected_action(&entries, "tile"),
            Some(NamedAction::SetLayout(LayoutCommand::Tile))
        );
        assert_eq!(selected_action(&entries, "grid"), None);
        assert_eq!(selected_action(&entries, "A translated label"), None);
    }

    #[test]
    fn chooser_uses_cursor_placement_and_preselects_current_layout() {
        let command = menu_command(LayoutCommand::Grid);
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert!(args.contains(&"--follow-cursor"));
        assert!(!args.contains(&"--position"));
        assert!(args.windows(2).any(|pair| pair == ["--preselect", "grid"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--placeholder", "Choose window layout"])
        );
    }
}
