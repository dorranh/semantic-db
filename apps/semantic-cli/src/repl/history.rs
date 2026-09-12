use std::{fs::OpenOptions, path::PathBuf};

use directories::BaseDirs;

use super::ReplEditor;

pub(super) struct Storage {
    path: Option<PathBuf>,
    warning: Option<String>,
}

impl Storage {
    pub fn new(disabled: bool, path: Option<PathBuf>) -> Self {
        if disabled {
            return Self {
                path: None,
                warning: None,
            };
        }
        let path = path.or_else(|| {
            BaseDirs::new().map(|dirs| {
                dirs.state_dir()
                    .unwrap_or_else(|| dirs.data_local_dir())
                    .join("semantic-db/history")
            })
        });
        let warning = path
            .is_none()
            .then(|| "cannot determine a history directory".into());
        Self { path, warning }
    }

    pub fn description(&self) -> String {
        self.path
            .as_ref()
            .map_or_else(|| "session only".into(), |p| p.display().to_string())
    }

    pub fn load(&mut self, editor: &mut ReplEditor) {
        if let Some(warning) = self.warning.take() {
            self.fail(warning);
        }
        let Some(path) = &self.path else {
            return;
        };
        let result = (|| -> super::Result<()> {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            // Create without truncation before Rustyline appends. This also avoids
            // two new sessions racing through FileHistory's create/save branch.
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(path)?;
            editor.load_history(path)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.fail(error.to_string());
        }
    }

    pub fn record(&mut self, editor: &mut ReplEditor, input: &str) {
        if input.trim().is_empty() {
            return;
        }
        let result = (|| -> super::Result<()> {
            if editor.add_history_entry(input)?
                && let Some(path) = &self.path
            {
                editor.append_history(path)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.fail(error.to_string());
        }
    }

    fn fail(&mut self, error: String) {
        eprintln!("Warning: history storage unavailable ({error}); using session history.");
        self.path = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyline::{
        Config,
        history::{History, SearchDirection},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "semantic-history-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn editor() -> ReplEditor {
        ReplEditor::with_config(
            Config::builder()
                .max_history_size(1_000)
                .unwrap()
                .history_ignore_dups(true)
                .unwrap()
                .build(),
        )
        .unwrap()
    }

    #[test]
    fn multiline_duplicates_disabled_and_limits() {
        let temp = Temp::new();
        let path = temp.0.join("history");
        let mut store = Storage::new(false, Some(path.clone()));
        let mut first = editor();
        store.load(&mut first);
        for input in ["SELECT\n 1;", "SELECT\n 1;", "", ".ask hello"] {
            store.record(&mut first, input);
        }
        let mut second = editor();
        store.load(&mut second);
        assert_eq!(second.history().len(), 2);
        assert_eq!(
            second
                .history()
                .get(0, SearchDirection::Forward)
                .unwrap()
                .unwrap()
                .entry,
            "SELECT\n 1;"
        );
        for n in 0..1_005 {
            store.record(&mut first, &format!("SELECT {n};"));
        }
        let mut reloaded = editor();
        store.load(&mut reloaded);
        assert_eq!(reloaded.history().len(), 1_000);
        assert_eq!(
            reloaded
                .history()
                .get(0, SearchDirection::Forward)
                .unwrap()
                .unwrap()
                .entry,
            "SELECT 5;"
        );
        let disabled_path = temp.0.join("disabled");
        let mut disabled = Storage::new(true, Some(disabled_path.clone()));
        disabled.load(&mut second);
        disabled.record(&mut second, "SELECT 99;");
        assert!(!disabled_path.exists());
        assert_eq!(second.history().len(), 3);
    }

    #[test]
    fn sessions_append_without_overwriting_each_other() {
        let temp = Temp::new();
        let path = temp.0.join("history");
        let mut a = Storage::new(false, Some(path.clone()));
        let mut b = Storage::new(false, Some(path));
        let (mut ea, mut eb) = (editor(), editor());
        a.load(&mut ea);
        b.load(&mut eb);
        a.record(&mut ea, "SELECT 1;");
        b.record(&mut eb, "SELECT 2;");
        a.record(&mut ea, "SELECT 3;");
        let mut all = editor();
        a.load(&mut all);
        assert_eq!(all.history().len(), 3);
    }

    #[test]
    fn storage_failure_keeps_session_history() {
        let temp = Temp::new();
        std::fs::create_dir_all(&temp.0).unwrap();
        let mut store = Storage::new(false, Some(temp.0.clone())); // directory is not a file
        let mut editor = editor();
        store.load(&mut editor);
        assert!(store.path.is_none());
        store.record(&mut editor, "SELECT 1;");
        assert_eq!(editor.history().len(), 1);
    }

    #[test]
    fn simultaneous_first_writers_preserve_all_entries() {
        let temp = Temp::new();
        let path = temp.0.join("history");
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for n in 0..4 {
                let path = &path;
                let barrier = &barrier;
                scope.spawn(move || {
                    let mut storage = Storage::new(false, Some(path.clone()));
                    let mut editor = editor();
                    storage.load(&mut editor);
                    barrier.wait();
                    storage.record(&mut editor, &format!("SELECT {n};"));
                    assert!(storage.path.is_some());
                });
            }
        });
        let mut storage = Storage::new(false, Some(path));
        let mut editor = editor();
        storage.load(&mut editor);
        assert_eq!(editor.history().len(), 4);
    }
}
