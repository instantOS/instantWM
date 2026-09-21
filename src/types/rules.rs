//! Window rule types.
//!
//! Types for defining and matching window rules.

use crate::types::TagMask;
use bincode::{Decode, Encode};
use serde::de::{Deserializer, Unexpected};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;
use std::time::Instant;

/// Floating behavior for window rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleFloat {
    /// Tiled window.
    #[default]
    Tiled,
    /// Floating window.
    Float,
    /// Centered floating window.
    FloatCenter,
    /// Fullscreen floating window.
    FloatFullscreen,
    /// Scratchpad window.
    Scratchpad,
}

/// Selects a monitor for a rule or IPC command.
///
/// One grammar shared by TOML rules and every `instantwmctl` surface that
/// names a monitor:
///
/// - `"any"` / [`MonitorSelector::Any`] — no preference
/// - `"focused"` — the currently selected monitor
/// - `"primary"` — the first monitor in layout order
/// - an integer (`2`) — the monitor at that **layout position** (0-based,
///   left-to-right in arrangement order, as shown by `instantwmctl monitor list`)
/// - anything else (`"DP-1"`) — exact match against the output name
///
/// Layout positions are the same integers accepted by `monitor switch` and
/// `window resize --monitor`; backend indices are deliberately *not* accepted
/// because they are unstable across backends and reconnects.
#[derive(Debug, Clone, PartialEq, Eq, Default, Encode, Decode)]
pub enum MonitorSelector {
    /// Place on any available monitor.
    #[default]
    Any,
    /// The currently selected (focused) monitor.
    Focused,
    /// The first monitor in layout order.
    Primary,
    /// The monitor at this layout position.
    Index(usize),
    /// The monitor with this output name (e.g. `"DP-1"`).
    Name(String),
}

impl MonitorSelector {
    /// The canonical string form of the selector keywords.
    fn keyword(&self) -> Option<&'static str> {
        match self {
            Self::Any => Some("any"),
            Self::Focused => Some("focused"),
            Self::Primary => Some("primary"),
            _ => None,
        }
    }
}

impl FromStr for MonitorSelector {
    type Err = std::convert::Infallible;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Ok(match raw {
            "any" => Self::Any,
            "focused" => Self::Focused,
            "primary" => Self::Primary,
            digits => match digits.parse::<usize>() {
                Ok(pos) => Self::Index(pos),
                Err(_) => Self::Name(digits.to_owned()),
            },
        })
    }
}

impl fmt::Display for MonitorSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index(pos) => write!(f, "{pos}"),
            Self::Name(name) => f.write_str(name),
            other => f.write_str(other.keyword().expect("non-keyword variants handled above")),
        }
    }
}

impl Serialize for MonitorSelector {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Index(pos) => serializer.serialize_u64(*pos as u64),
            Self::Name(name) => serializer.serialize_str(name),
            other => serializer
                .serialize_str(other.keyword().expect("non-keyword variants handled above")),
        }
    }
}

impl<'de> Deserialize<'de> for MonitorSelector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = MonitorSelector;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a monitor name, \"focused\", \"primary\", \"any\", or a layout position",
                )
            }

            fn visit_str<E: serde::de::Error>(self, raw: &str) -> Result<Self::Value, E> {
                MonitorSelector::from_str(raw).map_err(|unreachable| match unreachable {})
            }

            fn visit_u64<E: serde::de::Error>(self, pos: u64) -> Result<Self::Value, E> {
                usize::try_from(pos)
                    .map(MonitorSelector::Index)
                    .map_err(|_| E::invalid_value(Unexpected::Unsigned(pos), &self))
            }

            fn visit_i64<E: serde::de::Error>(self, pos: i64) -> Result<Self::Value, E> {
                usize::try_from(pos)
                    .map(MonitorSelector::Index)
                    .map_err(|_| E::invalid_value(Unexpected::Signed(pos), &self))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Exact placement for a rule-matched floating window.
///
/// Coordinates are **relative to the target monitor's work area** (the area
/// below the bar, where windows live), so a rule keeps placing the window in
/// the same spot when monitors are rearranged. Setting a geometry implies
/// floating placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(try_from = "RuleGeometryUnchecked")]
pub struct RuleGeometry {
    /// Distance from the left edge of the work area.
    pub x: i32,
    /// Distance from the top edge of the work area (below the bar).
    pub y: i32,
    /// Window width in pixels.
    pub width: i32,
    /// Window height in pixels.
    pub height: i32,
}

#[derive(Deserialize)]
struct RuleGeometryUnchecked {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl TryFrom<RuleGeometryUnchecked> for RuleGeometry {
    type Error = String;

    fn try_from(raw: RuleGeometryUnchecked) -> Result<Self, Self::Error> {
        let geometry = Self {
            x: raw.x,
            y: raw.y,
            width: raw.width,
            height: raw.height,
        };
        geometry
            .is_valid()
            .then_some(geometry)
            .ok_or_else(|| "geometry must have positive width and height".to_owned())
    }
}

impl RuleGeometry {
    /// Reject degenerate sizes at the CLI/config boundary.
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

impl FromStr for RuleGeometry {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
        let [x, y, width, height] = parts[..] else {
            return Err(format!(
                "invalid geometry '{raw}', expected X,Y,WIDTH,HEIGHT (e.g. 100,50,800,600)"
            ));
        };
        let parse = |label: &str, value: &str| -> Result<i32, String> {
            value
                .parse::<i32>()
                .map_err(|_| format!("invalid {label} '{value}' in geometry '{raw}'"))
        };
        let geometry = Self {
            x: parse("x", x)?,
            y: parse("y", y)?,
            width: parse("width", width)?,
            height: parse("height", height)?,
        };
        if !geometry.is_valid() {
            return Err(format!(
                "geometry '{raw}' must have positive width and height"
            ));
        }
        Ok(geometry)
    }
}

impl fmt::Display for RuleGeometry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{},{}", self.x, self.y, self.width, self.height)
    }
}

/// A window matching rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Window class to match.
    pub class: Option<Cow<'static, str>>,
    /// Window instance to match.
    pub instance: Option<Cow<'static, str>>,
    /// Window title to match.
    pub title: Option<Cow<'static, str>>,
    /// Tags to assign to matched windows.
    #[serde(default)]
    pub tags: TagMask,
    /// Floating behavior for matched windows.
    #[serde(default)]
    pub is_floating: Option<RuleFloat>,
    /// Monitor placement rule.
    #[serde(default)]
    pub monitor: MonitorSelector,
    /// Exact floating placement, relative to the target monitor's work area.
    /// Implies floating placement; an explicit `is_floating = "tiled"` is
    /// overridden.
    #[serde(default)]
    pub geometry: Option<RuleGeometry>,
    /// Matched windows are managed without a WM border.
    #[serde(default)]
    pub borderless: bool,
}

impl Rule {
    /// Check if this rule matches the window identifiers.
    pub fn matches(&self, class: &str, instance: &str, title: &str) -> bool {
        let title_match = self
            .title
            .as_ref()
            .map(|t| bytes_contains(title.as_bytes(), t))
            .unwrap_or(true);
        let class_match = self
            .class
            .as_ref()
            .map(|c| bytes_contains(class.as_bytes(), c))
            .unwrap_or(true);
        let instance_match = self
            .instance
            .as_ref()
            .map(|i| bytes_contains(instance.as_bytes(), i))
            .unwrap_or(true);

        title_match && class_match && instance_match
    }
}

#[inline]
fn bytes_contains(haystack: &[u8], needle: &str) -> bool {
    let nb = needle.as_bytes();
    // `windows(0)` panics; an empty matcher is degenerate, so treat it as
    // "matches everything" like `str::contains("")`.
    if nb.is_empty() {
        return true;
    }
    haystack.windows(nb.len()).any(|w| w == nb)
}

/// A rule queued at runtime to apply to the next matching window.
///
/// Distinct from config-loaded [`Rule`]s: every entry here has an absolute
/// deadline and is consumed on first match (and only on initial rule
/// application, never on property refresh). Expired entries are dropped
/// lazily by [`crate::client::rules`].
#[derive(Debug, Clone)]
pub struct PendingTmpRule {
    /// Unique id within the WM session, used by `--cancel` and the `--list`
    /// output.
    pub id: u64,
    /// The rule that will be applied on match.
    pub rule: Rule,
    /// Absolute deadline; the entry is dropped when `Instant::now()` passes
    /// this. The CLI always requires a positive TTL.
    pub deadline: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_rule() -> Rule {
        Rule {
            class: None,
            instance: None,
            title: None,
            tags: TagMask::EMPTY,
            is_floating: None,
            monitor: MonitorSelector::Any,
            geometry: None,
            borderless: false,
        }
    }

    #[test]
    fn empty_matcher_does_not_panic() {
        // `windows(0)` would panic, so an empty matcher must be handled.
        let rule = Rule {
            class: Some(Cow::Borrowed("")),
            ..empty_rule()
        };
        // Empty matcher matches everything rather than panicking.
        assert!(rule.matches("anything", "anything", "anything"));
    }

    #[test]
    fn selector_parses_keywords_indices_and_names() {
        assert_eq!(
            "any".parse::<MonitorSelector>().unwrap(),
            MonitorSelector::Any
        );
        assert_eq!(
            "focused".parse::<MonitorSelector>().unwrap(),
            MonitorSelector::Focused
        );
        assert_eq!(
            "primary".parse::<MonitorSelector>().unwrap(),
            MonitorSelector::Primary
        );
        assert_eq!(
            "2".parse::<MonitorSelector>().unwrap(),
            MonitorSelector::Index(2)
        );
        assert_eq!(
            "DP-1".parse::<MonitorSelector>().unwrap(),
            MonitorSelector::Name("DP-1".to_owned())
        );
        // Display round-trips every variant.
        for selector in [
            MonitorSelector::Any,
            MonitorSelector::Focused,
            MonitorSelector::Primary,
            MonitorSelector::Index(3),
            MonitorSelector::Name("HDMI-0".to_owned()),
        ] {
            assert_eq!(
                selector.to_string().parse::<MonitorSelector>().unwrap(),
                selector
            );
        }
    }

    #[test]
    fn selector_deserializes_from_toml_string_and_integer() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            monitor: MonitorSelector,
        }

        let from_string: Wrapper = toml::from_str("monitor = \"DP-1\"").expect("string selector");
        assert_eq!(
            from_string.monitor,
            MonitorSelector::Name("DP-1".to_owned())
        );

        let from_int: Wrapper = toml::from_str("monitor = 1").expect("integer selector");
        assert_eq!(from_int.monitor, MonitorSelector::Index(1));

        let keyword: Wrapper = toml::from_str("monitor = \"focused\"").expect("keyword selector");
        assert_eq!(keyword.monitor, MonitorSelector::Focused);

        let roundtrip = toml::to_string(&from_int).expect("serialize selector");
        assert_eq!(
            toml::from_str::<Wrapper>(&roundtrip).unwrap().monitor,
            MonitorSelector::Index(1)
        );

        let named = Wrapper {
            monitor: MonitorSelector::Name("DP-1".to_owned()),
        };
        let roundtrip = toml::to_string(&named).expect("serialize named selector");
        assert_eq!(
            toml::from_str::<Wrapper>(&roundtrip).unwrap().monitor,
            named.monitor
        );
    }

    #[test]
    fn geometry_parses_comma_quadruples() {
        let geometry = "100, 50, 800, 600".parse::<RuleGeometry>().unwrap();
        assert_eq!(
            geometry,
            RuleGeometry {
                x: 100,
                y: 50,
                width: 800,
                height: 600,
            }
        );
        assert_eq!(geometry.to_string(), "100,50,800,600");

        assert!("100,50,800".parse::<RuleGeometry>().is_err());
        assert!("100,50,0,600".parse::<RuleGeometry>().is_err());
        assert!("100,50,-5,600".parse::<RuleGeometry>().is_err());
        assert!("a,b,c,d".parse::<RuleGeometry>().is_err());
    }

    #[test]
    fn geometry_deserialization_rejects_degenerate_sizes() {
        assert!(toml::from_str::<RuleGeometry>("x = 0\ny = 0\nwidth = 0\nheight = 600\n").is_err());
        assert!(
            toml::from_str::<RuleGeometry>("x = 0\ny = 0\nwidth = 800\nheight = 600\n").is_ok()
        );
    }
}
