use crate::layouts::LayoutCommand;
use crate::types::{FocusFollowsMouseMode, MonitorDirection, StackDirection, ToggleAction};

/// Typed arguments of a named action, parsed once from their textual form
/// (config arrays, `instantwmctl action NAME ARGS...`) and rendered back for
/// display and IPC.
pub(super) trait ActionArgs: Sized {
    fn parse(args: &[String]) -> Result<Self, String>;
    fn render(&self) -> Vec<String>;
    fn usage() -> String;
}

/// One textual argument value.
trait ArgValue: Sized {
    fn parse_value(value: &str) -> Result<Self, String>;
    fn render_value(&self) -> String;
    fn usage() -> String;
}

macro_rules! value_enum_arg {
    ($($ty:ty),+) => {$(
        impl ArgValue for $ty {
            fn parse_value(value: &str) -> Result<Self, String> {
                <$ty as clap::ValueEnum>::from_str(value, true).map_err(|_| {
                    format!("invalid value '{value}'; expected {}", <Self as ArgValue>::usage())
                })
            }

            fn render_value(&self) -> String {
                clap::ValueEnum::to_possible_value(self)
                    .expect("no skipped variants")
                    .get_name()
                    .to_string()
            }

            fn usage() -> String {
                <$ty as clap::ValueEnum>::value_variants()
                    .iter()
                    .filter_map(clap::ValueEnum::to_possible_value)
                    .map(|value| value.get_name().to_string())
                    .collect::<Vec<_>>()
                    .join("|")
            }
        }
    )+};
}

value_enum_arg!(
    ToggleAction,
    MonitorDirection,
    StackDirection,
    FocusFollowsMouseMode
);

impl ArgValue for String {
    fn parse_value(value: &str) -> Result<Self, String> {
        Ok(value.to_string())
    }

    fn render_value(&self) -> String {
        self.clone()
    }

    fn usage() -> String {
        "NAME".to_string()
    }
}

impl ArgValue for i32 {
    fn parse_value(value: &str) -> Result<Self, String> {
        value
            .parse()
            .map_err(|_| format!("invalid value '{value}'; expected an integer"))
    }

    fn render_value(&self) -> String {
        self.to_string()
    }

    fn usage() -> String {
        "N".to_string()
    }
}

impl ArgValue for u32 {
    fn parse_value(value: &str) -> Result<Self, String> {
        value
            .parse()
            .map_err(|_| format!("invalid value '{value}'; expected a non-negative integer"))
    }

    fn render_value(&self) -> String {
        self.to_string()
    }

    fn usage() -> String {
        "N".to_string()
    }
}

impl ArgValue for LayoutCommand {
    fn parse_value(value: &str) -> Result<Self, String> {
        LayoutCommand::from_name(value).ok_or_else(|| {
            format!(
                "invalid layout '{value}'; expected {}",
                <Self as ArgValue>::usage()
            )
        })
    }

    fn render_value(&self) -> String {
        self.name().to_string()
    }

    fn usage() -> String {
        LayoutCommand::all()
            .iter()
            .map(|layout| layout.name())
            .collect::<Vec<_>>()
            .join("|")
    }
}

macro_rules! single_value_args {
    ($($ty:ty),+) => {$(
        impl ActionArgs for $ty {
            fn parse(args: &[String]) -> Result<Self, String> {
                match args {
                    [value] => Self::parse_value(value),
                    _ => Err(format!("expected 1 argument, got {}", args.len())),
                }
            }

            fn render(&self) -> Vec<String> {
                vec![self.render_value()]
            }

            fn usage() -> String {
                <Self as ArgValue>::usage()
            }
        }
    )+};
}

single_value_args!(
    String,
    u32,
    LayoutCommand,
    MonitorDirection,
    StackDirection,
    FocusFollowsMouseMode
);

impl<T: ArgValue> ActionArgs for Option<T> {
    fn parse(args: &[String]) -> Result<Self, String> {
        match args {
            [] => Ok(None),
            [value] => T::parse_value(value).map(Some),
            _ => Err(format!("expected at most 1 argument, got {}", args.len())),
        }
    }

    fn render(&self) -> Vec<String> {
        self.iter().map(ArgValue::render_value).collect()
    }

    fn usage() -> String {
        format!("[{}]", T::usage())
    }
}

/// A command line: program followed by its arguments.
impl ActionArgs for Vec<String> {
    fn parse(args: &[String]) -> Result<Self, String> {
        if args.is_empty() {
            return Err("expected a command".to_string());
        }
        Ok(args.to_vec())
    }

    fn render(&self) -> Vec<String> {
        self.clone()
    }

    fn usage() -> String {
        "COMMAND [ARG ...]".to_string()
    }
}

/// A `KEY VALUE` pair for the `config_set` action.
///
/// Both parts stay raw strings: the runtime-config layer parses values
/// through serde, exactly as `instantwmctl config set` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigAssignment {
    pub key: String,
    pub value: String,
}

impl ActionArgs for ConfigAssignment {
    fn parse(args: &[String]) -> Result<Self, String> {
        match args {
            [key, value] => Ok(Self {
                key: key.clone(),
                value: value.clone(),
            }),
            _ => Err(format!("expected KEY VALUE, got {} arguments", args.len())),
        }
    }

    fn render(&self) -> Vec<String> {
        vec![self.key.clone(), self.value.clone()]
    }

    fn usage() -> String {
        "KEY VALUE".to_string()
    }
}
