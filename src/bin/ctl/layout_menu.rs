//! Run the layout chooser in the client so waiting for a choice never blocks
//! either compositor backend.
use std::io::Write;
use std::process::{Command, Stdio};

use instantwm::LayoutMenuContext;
use instantwm::actions::NamedAction;
use instantwm::ipc_types::{IpcCommand, Response, TagCommand};
use instantwm::layouts::LayoutCommand;

use super::ipc;

struct Choice {
    value: String,
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

fn layout_choice(layout: LayoutCommand, active: LayoutCommand) -> Choice {
    let label = match layout {
        LayoutCommand::Tile => "Main window on left",
        LayoutCommand::Grid => "Grid",
        LayoutCommand::BottomStack => "Main window on top",
        LayoutCommand::Floating => "Floating windows",
        LayoutCommand::Maximized => "Maximized windows",
    };
    let current = layout == active;
    Choice {
        value: layout.name().into(),
        label: format!("{label}{}", if current { " (current)" } else { "" }),
        icon: layout.symbol(),
        color: if current { "green" } else { "blue" },
        // Choosing the current layout must not silently reset edits.
        action: (!current).then_some(NamedAction::SetLayout(layout)),
    }
}

fn choices(active: LayoutCommand) -> Vec<Choice> {
    // instantMENU fills columns top to bottom. Each tiling layout has its
    // reset to the right; floating and maximized share the final row since
    // they have no independent arrangement to reset.
    let layouts = [
        LayoutCommand::Tile,
        LayoutCommand::Grid,
        LayoutCommand::BottomStack,
    ];
    let mut entries: Vec<_> = layouts
        .iter()
        .map(|&layout| layout_choice(layout, active))
        .collect();
    entries.push(layout_choice(LayoutCommand::Floating, active));
    for layout in layouts {
        let choice = layout_choice(layout, active);
        entries.push(Choice {
            value: format!("reset-{}", layout.name()),
            label: format!(
                "Reset {}",
                choice.label.trim_end_matches(" (current)").to_lowercase()
            ),
            icon: "fa-arrow-rotate-left",
            color: "yellow",
            action: Some(NamedAction::ResetLayoutTo(layout)),
        });
    }
    entries.push(layout_choice(LayoutCommand::Maximized, active));
    entries
}

fn selected_action(entries: &[Choice], answer: &str) -> Option<NamedAction> {
    entries
        .iter()
        .find(|entry| entry.value == answer)
        .and_then(|entry| entry.action.clone())
}

/// Output-relative placement supplied by the WM when opening from the bar.
/// A direct CLI invocation without an anchor uses a centered menu.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct MenuAnchor {
    #[arg(long = "context", hide = true, value_parser = parse_context)]
    pub initial_context: Option<LayoutMenuContext>,
    #[arg(long)]
    pub monitor: Option<u32>,
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    pub x_offset: i32,
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    pub y_offset: i32,
}

fn parse_context(value: &str) -> Result<LayoutMenuContext, String> {
    serde_json::from_str(value).map_err(|error| format!("invalid menu context: {error}"))
}

fn menu_command(active: LayoutCommand, anchor: &MenuAnchor) -> Command {
    let mut command = Command::new("instantmenu");
    command
        .args([
            "--position",
            if anchor.monitor.is_some() {
                "top-left"
            } else {
                "center"
            },
            "--border-width",
            "4",
            "--width",
            "auto",
            "--line-height",
            "auto",
            "--columns",
            "2",
            "--lines",
            "4",
            "--placeholder",
            "Choose window layout",
            "--insensitive",
            "--reject-no-match",
            "--preselect",
            active.name(),
        ])
        .args([
            "--monitor",
            &anchor
                .monitor
                .map_or_else(|| "auto".into(), |index| index.to_string()),
        ])
        .args(["--x-offset", &anchor.x_offset.to_string()])
        .args(["--y-offset", &anchor.y_offset.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    command
}

// A chooser can stay open while the user switches workspaces. Dismiss a
// stale choice rather than applying it to a different tag or monitor.
fn context(ignore_version: bool) -> Result<LayoutMenuContext, String> {
    let (monitor, active) =
        match ipc::send(IpcCommand::LayoutStatus, ignore_version).map_err(|e| e.to_string())? {
            Response::LayoutStatus(status) => (
                status.monitor_id,
                LayoutCommand::from_name(&status.layout.name).ok_or("unknown active layout")?,
            ),
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
    Ok(LayoutMenuContext {
        monitor_id: monitor,
        tags,
        active,
    })
}

pub fn run(ignore_version: bool, anchor: MenuAnchor) -> Result<(), String> {
    let initial_context = match anchor.initial_context.clone() {
        Some(context) => context,
        None => context(ignore_version)?,
    };
    let active = initial_context.active;
    let entries = choices(active);
    let mut child = menu_command(active, &anchor)
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
    let current_context = context(ignore_version)?;
    if current_context.monitor_id != initial_context.monitor_id
        || current_context.tags != initial_context.tags
    {
        return Ok(());
    }
    if matches!(action, NamedAction::SetLayout(layout) if layout == current_context.active) {
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
            for layout in [
                LayoutCommand::Tile,
                LayoutCommand::Grid,
                LayoutCommand::BottomStack,
            ] {
                assert_eq!(
                    selected_action(&entries, &format!("reset-{}", layout.name())),
                    Some(NamedAction::ResetLayoutTo(layout))
                );
            }
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
    fn each_tiling_layout_has_its_reset_in_the_adjacent_column() {
        let entries = choices(LayoutCommand::Grid);
        assert_eq!(entries.len(), 8);
        for row in 0..3 {
            let Some(NamedAction::ResetLayoutTo(layout)) = entries[row + 4].action else {
                panic!("missing reset beside tiling layout");
            };
            assert_eq!(entries[row].value, layout.name());
        }
        assert_eq!(entries[3].value, "floating");
        assert_eq!(entries[7].value, "maximized");
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
    fn chooser_uses_explicit_anchor_without_probing_the_cursor() {
        let command = menu_command(
            LayoutCommand::Grid,
            &MenuAnchor {
                monitor: Some(1),
                x_offset: 135,
                y_offset: 30,
                ..Default::default()
            },
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert!(!args.contains(&"--follow-cursor"));
        for pair in [
            ["--position", "top-left"],
            ["--monitor", "1"],
            ["--x-offset", "135"],
            ["--y-offset", "30"],
        ] {
            assert!(args.windows(2).any(|args| args == pair));
        }
        assert!(args.windows(2).any(|pair| pair == ["--columns", "2"]));
        assert!(args.windows(2).any(|pair| pair == ["--lines", "4"]));
        assert!(args.windows(2).any(|pair| pair == ["--preselect", "grid"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--placeholder", "Choose window layout"])
        );
    }
}
