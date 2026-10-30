use super::{Shortcut, Trigger};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Saved {
    pub id: String,
    pub description: String,
    pub trigger: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Store {
    apps: BTreeMap<String, Vec<Saved>>,
    #[serde(skip)]
    path: Option<PathBuf>,
    #[serde(skip)]
    generations: BTreeMap<String, u64>,
}

impl Store {
    #[cfg(test)]
    pub(super) fn with_path(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }
    pub fn load() -> Self {
        let path = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| dirs::home_dir().map(|home| home.join(".local/state")))
            .map(|dir| dir.join("instantwm/global-shortcuts.toml"));
        let mut store = Self::default();
        if let Some(path) = &path {
            let loaded = std::fs::File::open(path).and_then(|file| {
                let mut text = String::new();
                file.take(1024 * 1024 + 1).read_to_string(&mut text)?;
                Ok(text)
            });
            match loaded {
                Ok(text) if text.len() <= 1024 * 1024 => match toml::from_str::<Self>(&text) {
                    Ok(saved) => store.apps = saved.apps,
                    Err(error) => {
                        log::warn!("global shortcuts: invalid saved preferences: {error}")
                    }
                },
                Ok(_) => log::warn!("global shortcuts: preferences exceed size limit"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log::warn!("global shortcuts: could not read preferences: {error}"),
            }
        }
        store.path = path;
        store
    }

    pub fn shortcuts(&self, app: &str) -> Vec<Shortcut> {
        if app.is_empty() {
            return Vec::new();
        }
        self.apps
            .get(app)
            .into_iter()
            .flatten()
            .map(|saved| Shortcut {
                id: saved.id.clone(),
                description: saved.description.clone(),
                preferred: None,
                trigger: saved
                    .trigger
                    .as_deref()
                    .and_then(|text| Trigger::parse(text).ok()),
            })
            .collect()
    }

    pub fn save(
        &mut self,
        app: &str,
        shortcuts: &[Shortcut],
        generation: u64,
    ) -> std::io::Result<()> {
        // Reconfiguration tasks can finish out of order; an older task must
        // never overwrite a newer grant for the same application.
        if self
            .generations
            .get(app)
            .is_some_and(|previous| *previous > generation)
        {
            return Ok(());
        }
        self.generations.insert(app.to_owned(), generation);
        // Unsandboxed clients without an app ID cannot share remembered grants.
        if app.is_empty() {
            return Ok(());
        }
        self.apps.insert(
            app.to_owned(),
            shortcuts
                .iter()
                .map(|shortcut| Saved {
                    id: shortcut.id.clone(),
                    description: shortcut.description.clone(),
                    trigger: shortcut.trigger.map(Trigger::text),
                })
                .collect(),
        );
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| std::io::Error::other("No state directory"))?;
        let dir = path.parent().unwrap();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let temporary = dir.join(format!(
            ".global-shortcuts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let result = (|| {
            let text = toml::to_string(self).map_err(std::io::Error::other)?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)?;
            std::fs::File::open(dir)?.sync_all()
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}
