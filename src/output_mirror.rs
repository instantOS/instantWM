//! Display mirroring policy shared by both backends.
//!
//! A "mirror" head presents its "source" output's region of the logical
//! desktop instead of owning a region itself, so the monitor layer sees one
//! monitor for both heads. How a backend realizes that differs (Wayland
//! projects the source's scene onto the mirror; X11 points the mirror's CRTC
//! at the source's framebuffer region), but the declaration rules live here.
//! The mapping is depth-1 by construction: a mirror target may never itself be
//! a declared mirror, which forbids chains (`A -> B -> C`) and cycles
//! (`A <-> B`) deterministically instead of at runtime.
//!
//! [`MirrorMap::build`] turns the raw `[monitors]` config entries into that
//! depth-1 map plus a stable list of [`MirrorConfigError`] diagnostics, and
//! [`sanitize_mirror_configs`] repairs a config map in place (clearing
//! rejected declarations and shadowed fields, retargeting relative anchors
//! that point at a mirror). [`MonitorPolicy`] is the sanitized result every
//! backend and the monitor layer read. [`fold_cloned_outputs`] merges outputs that
//! physically present the same region (such as `xrandr --same-as` clones)
//! into one logical output before the monitor layer runs.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::backend::BackendOutputInfo;
use crate::config::config_toml::{MirrorFit, MonitorConfig};
use crate::types::{MonitorPosition, RelativePosition};

/// A mirror declaration's resolved target: which output to follow and how
/// to fit content when framebuffers differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorTarget {
    /// The output whose presentation state this mirror follows.
    pub source: String,
    /// Letterbox (`Contain`) or crop (`Cover`) when the mirror's aspect
    /// ratio differs from its source's; identical-aspect mirrors ignore it.
    pub fit: MirrorFit,
}

/// Depth-1 mapping of mirror output name -> resolved [`MirrorTarget`].
///
/// Keys are declared mirrors, values name their sources. Because
/// [`MirrorMap::build`] rejects any target that is itself a declared mirror,
/// target names are never map keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirrorMap {
    /// mirror name -> resolved target; sorted for deterministic iteration.
    map: BTreeMap<String, MirrorTarget>,
}

/// Problem found while interpreting `mirror` declarations in monitor config.
///
/// Fatal variants invalidate the declaration and are cleared by
/// [`sanitize_mirror_configs`]; [`Self::ShadowedPresentation`] is a warning —
/// the mirror pair stays active and only the redundant presentation fields
/// are dropped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MirrorConfigError {
    #[error("output {output} declares an empty mirror target")]
    EmptyTarget { output: String },
    #[error("output {output} cannot mirror itself")]
    SelfReference { output: String },
    #[error("output {output} mirrors {target}, which is itself a mirror")]
    TargetIsMirror { output: String, target: String },
    #[error("the wildcard monitor entry cannot declare a mirror target")]
    WildcardDeclaration,
    #[error("output {output} cannot mirror the wildcard entry")]
    WildcardSource { output: String },
    #[error("output {output} mirrors {source}, which is not connected")]
    // Raw-ident declaration: thiserror otherwise infers a `source`-named
    // field to be the error source, which `String` cannot satisfy. The field
    // is still plainly named `source` everywhere else.
    SourceNotConnected { output: String, r#source: String },
    #[error("mirror output {output} ignores its own position and scale configuration")]
    ShadowedPresentation { output: String },
}

impl MirrorConfigError {
    /// Only shadowed presentation settings are non-fatal: the mirror pair
    /// itself remains valid.
    pub fn is_fatal(&self) -> bool {
        !matches!(self, Self::ShadowedPresentation { .. })
    }

    /// Config entry whose `mirror` value caused this error. [`Self::WildcardDeclaration`]
    /// is keyed by the wildcard entry `"*"`.
    pub fn declaration_key(&self) -> Option<&str> {
        match self {
            Self::EmptyTarget { output }
            | Self::SelfReference { output }
            | Self::TargetIsMirror { output, .. }
            | Self::WildcardSource { output }
            | Self::SourceNotConnected { output, .. }
            | Self::ShadowedPresentation { output } => Some(output),
            Self::WildcardDeclaration => Some("*"),
        }
    }
}

impl MirrorMap {
    /// Build the depth-1 mirror map from raw monitor config entries.
    ///
    /// Deterministic: declarations and errors are ordered by config key, and
    /// shadowed-presentation warnings come last. Structurally broken
    /// declarations (empty target, self reference, wildcard source or
    /// declaration) and chains/cycles are reported and excluded; a shadowed
    /// pair stays in the map. A declared `mirror_fit` travels with the target,
    /// defaulting to [`MirrorFit::Contain`].
    pub fn build(configs: &HashMap<String, MonitorConfig>) -> (Self, Vec<MirrorConfigError>) {
        let mut keys: Vec<&str> = configs.keys().map(String::as_str).collect();
        keys.sort_unstable();

        // Pass 1: keep only structurally sane, non-wildcard declarations.
        let mut errors = Vec::new();
        let mut raw: BTreeMap<String, MirrorTarget> = BTreeMap::new();
        for &key in &keys {
            let Some(config) = configs.get(key) else {
                continue;
            };
            let Some(declared) = config.mirror.as_deref() else {
                continue;
            };
            let target = declared.trim();
            if target.is_empty() {
                errors.push(MirrorConfigError::EmptyTarget {
                    output: key.to_string(),
                });
            } else if key == "*" {
                errors.push(MirrorConfigError::WildcardDeclaration);
            } else if target == "*" {
                errors.push(MirrorConfigError::WildcardSource {
                    output: key.to_string(),
                });
            } else if target == key {
                errors.push(MirrorConfigError::SelfReference {
                    output: key.to_string(),
                });
            } else {
                raw.insert(
                    key.to_string(),
                    MirrorTarget {
                        source: target.to_string(),
                        fit: config.mirror_fit.unwrap_or_default(),
                    },
                );
            }
        }

        // Pass 2: a target that is itself a declared mirror would form a
        // chain or a cycle; reject the declaration instead.
        let mut map = BTreeMap::new();
        for (output, target) in &raw {
            if raw.contains_key(&target.source) {
                errors.push(MirrorConfigError::TargetIsMirror {
                    output: output.clone(),
                    target: target.source.clone(),
                });
            } else {
                map.insert(output.clone(), target.clone());
            }
        }

        // Pass 3: a mirror head owns no desktop region, so its position and
        // scale are meaningless; warn but keep the pair. Mode and transform
        // describe the physical head and stay the head's own.
        for output in map.keys() {
            let config = &configs[output];
            if config.position.is_some() || config.scale.is_some() {
                errors.push(MirrorConfigError::ShadowedPresentation {
                    output: output.clone(),
                });
            }
        }

        (Self { map }, errors)
    }

    /// Source output `name` mirrors, if `name` is a declared mirror.
    pub fn source_of(&self, name: &str) -> Option<&str> {
        self.map.get(name).map(|target| target.source.as_str())
    }

    /// Fit policy `name` declares for its source, if `name` is a declared
    /// mirror.
    pub fn fit_of(&self, name: &str) -> Option<MirrorFit> {
        self.map.get(name).map(|target| target.fit)
    }

    /// Whether `name` is a declared mirror (a key of this map).
    pub fn contains_mirror(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    /// Iterate `(mirror, target)` pairs in deterministic name order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &MirrorTarget)> {
        self.map.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Pairs whose mirror and source heads are both active, i.e. the pairs a
    /// backend can currently realize. A mirror whose source is inactive
    /// behaves as an ordinary output.
    pub fn active_pairs<'a>(
        &'a self,
        is_active: impl Fn(&str) -> bool + 'a,
    ) -> impl Iterator<Item = (&'a str, &'a MirrorTarget)> + 'a {
        self.map
            .iter()
            .filter(move |(mirror, target)| is_active(mirror) && is_active(&target.source))
            .map(|(mirror, target)| (mirror.as_str(), target))
    }

    /// Construct a map directly from `(mirror, source)` pairs, bypassing
    /// [`Self::build`]'s chain/cycle rejection and defaulting every target's
    /// fit to [`MirrorFit::Contain`].
    #[cfg(test)]
    pub(crate) fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self::from_targets(pairs.into_iter().map(|(mirror, source)| {
            (
                mirror,
                MirrorTarget {
                    source,
                    fit: MirrorFit::Contain,
                },
            )
        }))
    }

    /// Construct a map from fully resolved targets, bypassing
    /// [`Self::build`]'s chain/cycle rejection. Test-only companion of
    /// [`Self::from_pairs`] for cases that exercise the fit policy.
    #[cfg(test)]
    pub(crate) fn from_targets(pairs: impl IntoIterator<Item = (String, MirrorTarget)>) -> Self {
        Self {
            map: pairs.into_iter().collect(),
        }
    }
}

/// Repair monitor config entries that conflict with mirroring.
///
/// 1. Build the mirror map (collecting diagnostics).
/// 2. Clear `mirror` on every fatally broken declaration.
/// 3. Clear the shadowed fields (position, scale) on valid mirror entries.
/// 4. Clear a `mirror_fit` whose entry has no effective `mirror` target; the
///    fit alone configures nothing, and a later mirror declaration must not
///    silently inherit a stale value.
/// 5. Retarget relative anchors (`right-of:DP-1`) that reference a mirror
///    onto that mirror's source, one hop only.
///
/// Returns the mirror map (unaffected by the repairs) and every error
/// [`MirrorMap::build`] produced, in stable order.
pub fn sanitize_mirror_configs(
    configs: &mut HashMap<String, MonitorConfig>,
) -> (MirrorMap, Vec<MirrorConfigError>) {
    let (map, errors) = MirrorMap::build(configs);

    for error in &errors {
        if !error.is_fatal() {
            continue;
        }
        if let Some(config) = error.declaration_key().and_then(|key| configs.get_mut(key)) {
            config.mirror = None;
        }
    }

    for error in &errors {
        if let MirrorConfigError::ShadowedPresentation { output } = error
            && let Some(config) = configs.get_mut(output)
        {
            config.position = None;
            config.scale = None;
        }
    }

    // A fit without an effective mirror target is inert; drop it (debug
    // level: an IPC `monitor set` may legitimately set it before the mirror).
    for (name, config) in configs.iter_mut() {
        let has_target = config
            .mirror
            .as_deref()
            .is_some_and(|declared| !declared.trim().is_empty());
        if !has_target && config.mirror_fit.is_some() {
            log::debug!("clearing mirror_fit of output {name}: no mirror target is declared");
            config.mirror_fit = None;
        }
    }

    for config in configs.values_mut() {
        let parsed = config.position.as_deref().and_then(MonitorPosition::parse);
        let Some(MonitorPosition::Relative { relation, output }) = parsed else {
            continue;
        };
        let Some(source) = map.source_of(&output) else {
            continue;
        };
        let relation_str = match relation {
            RelativePosition::LeftOf => "left-of",
            RelativePosition::RightOf => "right-of",
            RelativePosition::Above => "above",
            RelativePosition::Below => "below",
        };
        config.position = Some(format!("{relation_str}:{source}"));
    }

    (map, errors)
}

/// The effective monitor policy: `[monitors]` config repaired by
/// [`sanitize_mirror_configs`] together with its mirror map. Built once per
/// monitor config apply and stored in [`crate::core_state::DerivedState`].
#[derive(Debug, Clone, Default)]
pub struct MonitorPolicy {
    pub configs: HashMap<String, MonitorConfig>,
    pub mirrors: MirrorMap,
}

impl MonitorPolicy {
    /// Sanitize `configs`, logging every diagnostic (`error!` for fatal,
    /// `warn!` otherwise).
    pub fn new(configs: &HashMap<String, MonitorConfig>) -> Self {
        let mut configs = configs.clone();
        let (mirrors, errors) = sanitize_mirror_configs(&mut configs);
        for error in errors {
            if error.is_fatal() {
                log::error!("{error}");
            } else {
                log::warn!("{error}");
            }
        }
        Self { configs, mirrors }
    }

    /// The entry governing output `name`: its named entry, else the wildcard.
    pub fn effective(&self, name: &str) -> Option<&MonitorConfig> {
        self.configs.get(name).or_else(|| self.configs.get("*"))
    }

    pub fn is_explicitly_disabled(&self, name: &str) -> bool {
        self.effective(name)
            .is_some_and(|config| config.enable == Some(false))
    }
}

/// Merge outputs that physically present the same desktop region into one
/// logical output per region.
///
/// An output whose rectangle lies inside another output's rectangle shows a
/// subset of that output's pixels (one X11 framebuffer, or overlapping regions
/// of the Wayland space), so it becomes one of the containing output's
/// [`BackendOutputInfo::mirrors`]. This covers external `xrandr --same-as`
/// clones, which never mention `mirror` at all. Among identical rectangles the
/// survivor is, in order: not a declared mirror (a source keeps its identity
/// over its mirror), an existing monitor name from `preferred` (stable
/// identity), then the lexicographically smallest name. Survivors keep their
/// relative order.
pub fn fold_cloned_outputs(
    outputs: Vec<BackendOutputInfo>,
    mirrors: &MirrorMap,
    preferred: &HashSet<String>,
) -> Vec<BackendOutputInfo> {
    let rank = |output: &BackendOutputInfo| {
        (
            mirrors.contains_mirror(&output.name),
            !preferred.contains(&output.name),
            output.name.clone(),
        )
    };
    let dominates = |host: &BackendOutputInfo, output: &BackendOutputInfo| {
        host.rect.contains_rect(&output.rect)
            && (host.rect != output.rect || rank(host) < rank(output))
    };
    let survives: Vec<bool> = outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            !outputs
                .iter()
                .enumerate()
                .any(|(other, host)| other != index && dominates(host, output))
        })
        .collect();

    // Containment is transitive and rank is a strict order, so every folded
    // output lies inside at least one survivor. The tightest one hosts it.
    let mut folded_into: Vec<Vec<usize>> = vec![Vec::new(); outputs.len()];
    for (index, output) in outputs.iter().enumerate() {
        if survives[index] {
            continue;
        }
        let host = (0..outputs.len())
            .filter(|&candidate| survives[candidate])
            .filter(|&candidate| outputs[candidate].rect.contains_rect(&output.rect))
            .min_by_key(|&candidate| (outputs[candidate].rect.area(), rank(&outputs[candidate])));
        if let Some(host) = host {
            folded_into[host].push(index);
        }
    }

    let names: Vec<(String, Vec<String>)> = outputs
        .iter()
        .map(|output| (output.name.clone(), output.mirrors.clone()))
        .collect();
    outputs
        .into_iter()
        .enumerate()
        .filter(|(index, _)| survives[*index])
        .map(|(index, mut output)| {
            for &folded in &folded_into[index] {
                let (name, mirrors) = &names[folded];
                log::debug!("folding output {name} into {}", output.name);
                output.mirrors.push(name.clone());
                output.mirrors.extend(mirrors.iter().cloned());
            }
            output.mirrors.sort();
            output.mirrors.dedup();
            output
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendOutputInfo, BackendVrrSupport};
    use crate::types::Rect;

    type Configs = HashMap<String, MonitorConfig>;

    fn mirror_config(target: &str) -> MonitorConfig {
        MonitorConfig {
            mirror: Some(target.to_string()),
            ..MonitorConfig::default()
        }
    }

    fn config(entry: (&str, MonitorConfig)) -> Configs {
        [(entry.0.to_string(), entry.1)].into_iter().collect()
    }

    fn output(name: &str, rect: Rect) -> BackendOutputInfo {
        BackendOutputInfo {
            name: name.to_string(),
            rect,
            scale: 1.0,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
            mirrors: Vec::new(),
        }
    }

    fn names(outputs: &[BackendOutputInfo]) -> Vec<&str> {
        outputs.iter().map(|output| output.name.as_str()).collect()
    }

    #[test]
    fn valid_pair_builds_without_errors() {
        let (map, errors) = MirrorMap::build(&config(("DP-1", mirror_config("eDP-1"))));

        assert_eq!(errors, Vec::new());
        assert!(map.contains_mirror("DP-1"));
        assert!(!map.contains_mirror("eDP-1"));
        assert_eq!(map.source_of("DP-1"), Some("eDP-1"));
        assert_eq!(map.source_of("eDP-1"), None);
        assert!(!map.is_empty());
        assert_eq!(
            map.iter().collect::<Vec<_>>(),
            vec![(
                &"DP-1".to_string(),
                &MirrorTarget {
                    source: "eDP-1".to_string(),
                    fit: MirrorFit::Contain,
                }
            )]
        );
    }

    #[test]
    fn build_carries_the_declared_fit_and_defaults_to_contain() {
        // No `mirror_fit`: the default is contain.
        let (map, errors) = MirrorMap::build(&config(("DP-1", mirror_config("eDP-1"))));
        assert!(errors.is_empty());
        assert_eq!(map.fit_of("DP-1"), Some(MirrorFit::Contain));

        // An explicit fit travels with the target untouched.
        let configs = config((
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".into()),
                mirror_fit: Some(MirrorFit::Cover),
                ..MonitorConfig::default()
            },
        ));
        let (map, errors) = MirrorMap::build(&configs);
        assert_eq!(errors, Vec::new());
        assert_eq!(
            map.iter().collect::<Vec<_>>(),
            vec![(
                &"DP-1".to_string(),
                &MirrorTarget {
                    source: "eDP-1".to_string(),
                    fit: MirrorFit::Cover,
                }
            )]
        );
    }

    #[test]
    fn empty_map_reports_itself() {
        let map = MirrorMap::default();
        assert!(map.is_empty());
        assert_eq!(map.iter().count(), 0);
        assert!(!map.contains_mirror("DP-1"));
    }

    #[test]
    fn self_reference_is_fatal_and_excluded() {
        let (map, errors) = MirrorMap::build(&config(("DP-1", mirror_config("DP-1"))));

        assert_eq!(
            errors,
            vec![MirrorConfigError::SelfReference {
                output: "DP-1".into()
            }]
        );
        assert!(map.is_empty());
    }

    #[test]
    fn empty_target_is_fatal_and_excluded() {
        let (map, errors) = MirrorMap::build(&config(("DP-1", mirror_config("  "))));

        assert_eq!(
            errors,
            vec![MirrorConfigError::EmptyTarget {
                output: "DP-1".into()
            }]
        );
        assert!(map.is_empty());
    }

    #[test]
    fn wildcard_declaration_is_fatal() {
        let (map, errors) = MirrorMap::build(&config(("*", mirror_config("eDP-1"))));

        assert_eq!(errors, vec![MirrorConfigError::WildcardDeclaration]);
        assert!(map.is_empty());
    }

    #[test]
    fn wildcard_source_is_fatal_and_excluded() {
        let (map, errors) = MirrorMap::build(&config(("DP-1", mirror_config("*"))));

        assert_eq!(
            errors,
            vec![MirrorConfigError::WildcardSource {
                output: "DP-1".into()
            }]
        );
        assert!(map.is_empty());
    }

    #[test]
    fn chain_keeps_only_the_last_leg() {
        let configs: Configs = [
            ("A".to_string(), mirror_config("B")),
            ("B".to_string(), mirror_config("C")),
        ]
        .into_iter()
        .collect();
        let (map, errors) = MirrorMap::build(&configs);

        assert_eq!(
            errors,
            vec![MirrorConfigError::TargetIsMirror {
                output: "A".into(),
                target: "B".into(),
            }]
        );
        assert!(!map.contains_mirror("A"));
        assert_eq!(map.source_of("B"), Some("C"));
    }

    #[test]
    fn cycle_rejects_both_legs() {
        let configs: Configs = [
            ("A".to_string(), mirror_config("B")),
            ("B".to_string(), mirror_config("A")),
        ]
        .into_iter()
        .collect();
        let (map, errors) = MirrorMap::build(&configs);

        assert_eq!(
            errors,
            vec![
                MirrorConfigError::TargetIsMirror {
                    output: "A".into(),
                    target: "B".into(),
                },
                MirrorConfigError::TargetIsMirror {
                    output: "B".into(),
                    target: "A".into(),
                },
            ]
        );
        assert!(map.is_empty());
    }

    #[test]
    fn shadowed_presentation_warns_but_keeps_the_pair() {
        let configs = config((
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".into()),
                position: Some("0,0".into()),
                ..MonitorConfig::default()
            },
        ));
        let (map, errors) = MirrorMap::build(&configs);

        assert_eq!(
            errors,
            vec![MirrorConfigError::ShadowedPresentation {
                output: "DP-1".into()
            }]
        );
        assert_eq!(map.source_of("DP-1"), Some("eDP-1"));
    }

    #[test]
    fn fatal_flag_and_declaration_keys() {
        assert!(
            !MirrorConfigError::ShadowedPresentation {
                output: "DP-1".into()
            }
            .is_fatal()
        );
        assert!(
            MirrorConfigError::SelfReference {
                output: "DP-1".into()
            }
            .is_fatal()
        );
        assert!(MirrorConfigError::WildcardDeclaration.is_fatal());

        assert_eq!(
            MirrorConfigError::WildcardDeclaration.declaration_key(),
            Some("*")
        );
        assert_eq!(
            MirrorConfigError::EmptyTarget {
                output: "DP-1".into()
            }
            .declaration_key(),
            Some("DP-1")
        );
        assert_eq!(
            MirrorConfigError::SourceNotConnected {
                output: "DP-1".into(),
                source: "HDMI-1".into(),
            }
            .declaration_key(),
            Some("DP-1")
        );
        assert_eq!(
            MirrorConfigError::TargetIsMirror {
                output: "DP-1".into(),
                target: "HDMI-1".into(),
            }
            .declaration_key(),
            Some("DP-1")
        );
    }

    #[test]
    fn sanitize_clears_fatal_declarations() {
        let mut configs: Configs = [
            ("DP-1".to_string(), mirror_config("DP-1")),
            ("HDMI-1".to_string(), mirror_config("*")),
            ("*".to_string(), mirror_config("eDP-1")),
            ("HDMI-2".to_string(), mirror_config("eDP-1")),
        ]
        .into_iter()
        .collect();

        let (_, errors) = sanitize_mirror_configs(&mut configs);

        assert!(errors.iter().all(MirrorConfigError::is_fatal));
        assert_eq!(configs["DP-1"].mirror, None);
        assert_eq!(configs["HDMI-1"].mirror, None);
        assert_eq!(configs["*"].mirror, None);
        assert_eq!(configs["HDMI-2"].mirror, Some("eDP-1".to_string()));
    }

    #[test]
    fn sanitize_clears_shadowed_presentation_but_keeps_the_mirror() {
        let mut configs = config((
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".into()),
                position: Some("1920,0".into()),
                resolution: Some("2560x1440".into()),
                refresh_rate: Some(144.0),
                scale: Some(2.0),
                transform: Some(crate::config::config_toml::Transform::Rotate90),
                mirror_fit: Some(MirrorFit::Cover),
                ..MonitorConfig::default()
            },
        ));

        let (_, errors) = sanitize_mirror_configs(&mut configs);

        assert_eq!(
            errors,
            vec![MirrorConfigError::ShadowedPresentation {
                output: "DP-1".into()
            }]
        );
        let sanitized = &configs["DP-1"];
        assert_eq!(sanitized.mirror, Some("eDP-1".to_string()));
        assert_eq!(sanitized.position, None);
        assert_eq!(sanitized.scale, None);
        // Mode and transform describe the physical head, which keeps them.
        assert_eq!(sanitized.resolution.as_deref(), Some("2560x1440"));
        assert_eq!(sanitized.refresh_rate, Some(144.0));
        assert_eq!(
            sanitized.transform,
            Some(crate::config::config_toml::Transform::Rotate90)
        );
        assert_eq!(sanitized.mirror_fit, Some(MirrorFit::Cover));
    }

    #[test]
    fn sanitize_clears_a_fit_without_a_mirror_target() {
        let mut configs: Configs = [
            (
                "DP-1".to_string(),
                MonitorConfig {
                    mirror_fit: Some(MirrorFit::Cover),
                    ..MonitorConfig::default()
                },
            ),
            (
                "HDMI-1".to_string(),
                MonitorConfig {
                    // Whitespace-only is no effective target either.
                    mirror: Some("  ".into()),
                    mirror_fit: Some(MirrorFit::Cover),
                    ..MonitorConfig::default()
                },
            ),
        ]
        .into_iter()
        .collect();

        let (_, errors) = sanitize_mirror_configs(&mut configs);

        // The whitespace declaration is reported, the fitless entry is not.
        assert_eq!(
            errors,
            vec![MirrorConfigError::EmptyTarget {
                output: "HDMI-1".into()
            }]
        );
        assert_eq!(configs["DP-1"].mirror_fit, None);
        assert_eq!(configs["HDMI-1"].mirror_fit, None);
    }

    #[test]
    fn sanitize_keeps_the_fit_of_a_valid_mirror() {
        let mut configs = config((
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".into()),
                mirror_fit: Some(MirrorFit::Cover),
                ..MonitorConfig::default()
            },
        ));

        let (_, errors) = sanitize_mirror_configs(&mut configs);

        assert_eq!(errors, Vec::new());
        assert_eq!(configs["DP-1"].mirror_fit, Some(MirrorFit::Cover));
    }

    #[test]
    fn sanitize_retargets_relative_anchors_onto_the_source() {
        let mut configs: Configs = [
            (
                "DP-1".to_string(),
                MonitorConfig {
                    mirror: Some("eDP-1".into()),
                    ..MonitorConfig::default()
                },
            ),
            (
                "HDMI-1".to_string(),
                MonitorConfig {
                    position: Some("right-of:DP-1".into()),
                    ..MonitorConfig::default()
                },
            ),
        ]
        .into_iter()
        .collect();

        sanitize_mirror_configs(&mut configs);

        assert_eq!(
            configs["HDMI-1"].position.as_deref(),
            Some("right-of:eDP-1")
        );
        // Anchors that don't reference a mirror are left alone.
        let mut untouched = config((
            "HDMI-2",
            MonitorConfig {
                position: Some("below:eDP-1".into()),
                ..MonitorConfig::default()
            },
        ));
        sanitize_mirror_configs(&mut untouched);
        assert_eq!(untouched["HDMI-2"].position.as_deref(), Some("below:eDP-1"));
    }

    #[test]
    fn monitor_policy_repairs_a_clone_and_keeps_valid_mirrors() {
        let configs: Configs = [
            ("DP-1".to_string(), mirror_config("DP-1")),
            ("HDMI-1".to_string(), mirror_config("eDP-1")),
        ]
        .into_iter()
        .collect();

        let policy = MonitorPolicy::new(&configs);

        assert_eq!(configs["DP-1"].mirror, Some("DP-1".to_string()));
        assert_eq!(policy.configs["DP-1"].mirror, None);
        assert_eq!(policy.configs["HDMI-1"].mirror, Some("eDP-1".to_string()));
        assert_eq!(policy.mirrors.source_of("HDMI-1"), Some("eDP-1"));
        assert!(!policy.mirrors.contains_mirror("DP-1"));
    }

    #[test]
    fn named_monitor_policy_shadows_wildcard_disable() {
        let configs: Configs = [
            (
                "*".to_string(),
                MonitorConfig {
                    enable: Some(true),
                    ..MonitorConfig::default()
                },
            ),
            (
                "DP-1".to_string(),
                MonitorConfig {
                    enable: Some(false),
                    ..MonitorConfig::default()
                },
            ),
        ]
        .into_iter()
        .collect();
        let policy = MonitorPolicy::new(&configs);

        assert!(policy.is_explicitly_disabled("DP-1"));
        assert_eq!(
            policy.effective("HDMI-1").and_then(|config| config.enable),
            Some(true)
        );
    }

    #[test]
    fn mode_and_transform_on_a_mirror_are_not_shadowed() {
        let configs = config((
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".into()),
                resolution: Some("1920x1080".into()),
                transform: Some(crate::config::config_toml::Transform::Rotate90),
                ..MonitorConfig::default()
            },
        ));
        let (_, errors) = MirrorMap::build(&configs);
        assert_eq!(errors, Vec::new());
    }

    #[test]
    fn active_pairs_require_both_heads() {
        let configs: Configs = [
            ("DP-1".to_string(), mirror_config("eDP-1")),
            ("HDMI-1".to_string(), mirror_config("eDP-1")),
        ]
        .into_iter()
        .collect();
        let (map, _) = MirrorMap::build(&configs);

        let active: HashSet<&str> = ["DP-1", "eDP-1"].into_iter().collect();
        let pairs: Vec<_> = map
            .active_pairs(|name| active.contains(name))
            .map(|(mirror, target)| (mirror, target.source.as_str()))
            .collect();
        assert_eq!(pairs, vec![("DP-1", "eDP-1")]);

        assert_eq!(map.active_pairs(|name| name != "eDP-1").count(), 0);
    }

    #[test]
    fn disjoint_outputs_are_not_folded() {
        let outputs = vec![
            output("eDP-1", Rect::new(0, 0, 1920, 1080)),
            output("DP-1", Rect::new(1920, 0, 2560, 1440)),
        ];
        let folded = fold_cloned_outputs(outputs, &MirrorMap::default(), &HashSet::new());

        assert_eq!(names(&folded), vec!["eDP-1", "DP-1"]);
        assert!(folded.iter().all(|output| output.mirrors.is_empty()));
    }

    #[test]
    fn backend_reported_mirrors_are_kept() {
        let mut source = output("eDP-1", Rect::new(0, 0, 1920, 1080));
        source.mirrors = vec!["DP-1".into()];
        let folded = fold_cloned_outputs(vec![source], &MirrorMap::default(), &HashSet::new());

        assert_eq!(folded[0].mirrors, vec!["DP-1".to_string()]);
    }

    #[test]
    fn equal_rect_fold_prefers_preferred_then_lexicographic() {
        let rect = Rect::new(0, 0, 1920, 1080);
        let empty = MirrorMap::default();

        // No preference: lexicographically smallest name survives.
        let outputs = vec![
            output("HDMI-1", rect),
            output("DP-1", rect),
            output("eDP-1", Rect::new(1920, 0, 1920, 1080)),
        ];
        let folded = fold_cloned_outputs(outputs, &empty, &HashSet::new());
        assert_eq!(names(&folded), vec!["DP-1", "eDP-1"]);
        assert_eq!(folded[0].mirrors, vec!["HDMI-1".to_string()]);

        // Preferred survivor wins over the lexicographic one.
        let outputs = vec![output("DP-1", rect), output("HDMI-1", rect)];
        let preferred: HashSet<String> = ["HDMI-1".to_string()].into_iter().collect();
        let folded = fold_cloned_outputs(outputs, &empty, &preferred);
        assert_eq!(names(&folded), vec!["HDMI-1"]);

        // Several preferred: lexicographically smallest preferred wins.
        let outputs = vec![
            output("HDMI-1", rect),
            output("DP-2", rect),
            output("DP-1", rect),
        ];
        let preferred: HashSet<String> = ["DP-1".to_string(), "DP-2".to_string()]
            .into_iter()
            .collect();
        let folded = fold_cloned_outputs(outputs, &empty, &preferred);
        assert_eq!(names(&folded), vec!["DP-1"]);
        assert_eq!(
            folded[0].mirrors,
            vec!["DP-2".to_string(), "HDMI-1".to_string()]
        );
    }

    #[test]
    fn a_source_keeps_its_identity_over_its_declared_mirror() {
        // DP-1 was a monitor of its own before it started mirroring eDP-1.
        let (map, _) = MirrorMap::build(&config(("DP-1", mirror_config("eDP-1"))));
        let rect = Rect::new(0, 0, 1920, 1080);
        let preferred: HashSet<String> = ["DP-1".to_string()].into_iter().collect();

        let folded = fold_cloned_outputs(
            vec![output("DP-1", rect), output("eDP-1", rect)],
            &map,
            &preferred,
        );

        assert_eq!(names(&folded), vec!["eDP-1"]);
        assert_eq!(folded[0].mirrors, vec!["DP-1".to_string()]);
    }

    #[test]
    fn contained_outputs_fold_into_the_tightest_container() {
        // X11 crop mirror: a centered 1920x1080 CRTC inside a 2560x1440 one.
        // Nested clones fold into the tightest survivor containing them.
        let outputs = vec![
            output("HDMI-1", Rect::new(3840, 0, 1280, 1024)),
            output("eDP-1", Rect::new(0, 0, 2560, 1440)),
            output("DP-1", Rect::new(320, 180, 1920, 1080)),
            output("DP-2", Rect::new(320, 180, 1920, 1080)),
        ];
        let folded = fold_cloned_outputs(outputs, &MirrorMap::default(), &HashSet::new());

        assert_eq!(names(&folded), vec!["HDMI-1", "eDP-1"]);
        assert_eq!(
            folded[1].mirrors,
            vec!["DP-1".to_string(), "DP-2".to_string()]
        );
    }

    #[test]
    fn partially_overlapping_outputs_stay_separate() {
        let outputs = vec![
            output("eDP-1", Rect::new(0, 0, 1920, 1080)),
            output("DP-1", Rect::new(960, 0, 1920, 1080)),
        ];
        let folded = fold_cloned_outputs(outputs, &MirrorMap::default(), &HashSet::new());

        assert_eq!(names(&folded), vec!["eDP-1", "DP-1"]);
    }
}
