//! Per-agent tab targeting. Every page command acts on one explicit tab:
//! `--tab <targetId>` names it directly, `--session <key>` binds the key to a
//! dedicated background tab persisted in the session store.

use crate::cdp::TargetJson;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub const MISSING_SELECTOR: &str = "No tab selected. Pass --session <key> (or BROWSER_CLI_SESSION) to use a dedicated tab, \
     or --tab <targetId> (or BROWSER_CLI_TAB). Run `browser-cli tabs list` to see target IDs.";

#[derive(Clone, Debug, PartialEq)]
pub enum TabSelector {
    Tab(String),
    Session(String),
}

pub fn tab_selector(tab: Option<String>, session: Option<String>) -> Result<Option<TabSelector>> {
    let tab = tab.filter(|value| !value.trim().is_empty());
    let session = session.filter(|value| !value.trim().is_empty());
    match (tab, session) {
        (Some(_), Some(_)) => bail!("Pass either --tab or --session, not both"),
        (Some(tab), None) => Ok(Some(TabSelector::Tab(tab))),
        (None, Some(session)) => Ok(Some(TabSelector::Session(session))),
        (None, None) => Ok(None),
    }
}

#[derive(Debug, PartialEq)]
pub enum TabChoice {
    Use(TargetJson),
    /// The session has no live tab; create a background tab and bind it.
    CreateForSession,
}

pub fn choose_tab(
    targets: &[TargetJson],
    selector: &TabSelector,
    bound_target_id: Option<&str>,
) -> Result<TabChoice> {
    match selector {
        TabSelector::Tab(id) => find_target(targets, id)
            .map(TabChoice::Use)
            .with_context(|| format!("Tab {id} is not open. Run `browser-cli tabs list`.")),
        TabSelector::Session(_) => Ok(bound_target_id
            .and_then(|id| find_target(targets, id))
            .map_or(TabChoice::CreateForSession, TabChoice::Use)),
    }
}

fn find_target(targets: &[TargetJson], id: &str) -> Option<TargetJson> {
    targets.iter().find(|target| target.id == id).cloned()
}

pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn default_location() -> Result<Self> {
        Ok(Self::new(session_store_dir(
            std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        )?))
    }

    pub fn bound_target(&self, session: &str) -> Result<Option<String>> {
        match std::fs::read_to_string(self.path(session)) {
            Ok(id) => Ok(Some(id.trim().to_string()).filter(|id| !id.is_empty())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("Failed to read browser-cli session binding"),
        }
    }

    pub fn bind(&self, session: &str, target_id: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir).context("Failed to create browser-cli session store")?;
        let path = self.path(session);
        let temp = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temp, target_id).context("Failed to write browser-cli session binding")?;
        std::fs::rename(&temp, &path).context("Failed to save browser-cli session binding")
    }

    fn path(&self, session: &str) -> PathBuf {
        self.dir.join(session_file_name(session))
    }
}

fn session_store_dir(xdg_state_home: Option<PathBuf>, home: Option<PathBuf>) -> Result<PathBuf> {
    let state = match xdg_state_home.filter(|dir| dir.is_absolute()) {
        Some(dir) => dir,
        None => home
            .context("Cannot locate session store: HOME is not set")?
            .join(".local/state"),
    };
    Ok(state.join("browser-cli/sessions"))
}

/// Hex-encode the key so any session string maps to one safe file name.
fn session_file_name(session: &str) -> String {
    session.bytes().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str, url: &str) -> TargetJson {
        TargetJson {
            id: id.to_string(),
            title: String::new(),
            url: url.to_string(),
            r#type: "page".to_string(),
            web_socket_debugger_url: Some(format!("ws://127.0.0.1:9222/devtools/page/{id}")),
        }
    }

    fn temp_store() -> SessionStore {
        let dir = std::env::temp_dir().join(format!(
            "browser-cli-sessions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        SessionStore::new(dir)
    }

    #[test]
    fn selector_requires_exactly_one_of_tab_or_session() {
        assert_eq!(tab_selector(None, None).unwrap(), None);
        assert_eq!(
            tab_selector(Some(" ".into()), Some("".into())).unwrap(),
            None
        );
        assert_eq!(
            tab_selector(Some("ABC".into()), None).unwrap(),
            Some(TabSelector::Tab("ABC".into()))
        );
        assert_eq!(
            tab_selector(None, Some("s1:main".into())).unwrap(),
            Some(TabSelector::Session("s1:main".into()))
        );
        assert!(tab_selector(Some("ABC".into()), Some("s1".into())).is_err());
    }

    #[test]
    fn explicit_tab_uses_that_target_even_when_another_is_first() {
        let targets = [
            target("OTHER", "https://a.example/"),
            target("MINE", "https://b.example/"),
        ];
        let choice = choose_tab(&targets, &TabSelector::Tab("MINE".into()), None).unwrap();
        assert_eq!(choice, TabChoice::Use(targets[1].clone()));
    }

    #[test]
    fn explicit_tab_that_closed_fails_instead_of_using_another_tab() {
        let targets = [target("OTHER", "https://a.example/")];
        let error = choose_tab(&targets, &TabSelector::Tab("GONE".into()), None).unwrap_err();
        assert!(error.to_string().contains("Tab GONE is not open"));
    }

    #[test]
    fn session_reuses_its_bound_live_tab() {
        let targets = [
            target("OTHER", "https://a.example/"),
            target("MINE", "https://b.example/"),
        ];
        let choice =
            choose_tab(&targets, &TabSelector::Session("s1".into()), Some("MINE")).unwrap();
        assert_eq!(choice, TabChoice::Use(targets[1].clone()));
    }

    #[test]
    fn session_without_live_binding_creates_a_new_tab() {
        let targets = [target("OTHER", "https://a.example/")];
        let session = TabSelector::Session("s1".into());
        assert_eq!(
            choose_tab(&targets, &session, None).unwrap(),
            TabChoice::CreateForSession
        );
        assert_eq!(
            choose_tab(&targets, &session, Some("CLOSED")).unwrap(),
            TabChoice::CreateForSession
        );
    }

    #[test]
    fn session_store_keeps_independent_bindings_per_session() {
        let store = temp_store();
        assert_eq!(store.bound_target("sess-a:main").unwrap(), None);
        store.bind("sess-a:main", "TAB-A").unwrap();
        store.bind("sess-b:agent_7", "TAB-B").unwrap();
        store.bind("../escape/attempt", "TAB-C").unwrap();
        assert_eq!(
            store.bound_target("sess-a:main").unwrap(),
            Some("TAB-A".into())
        );
        assert_eq!(
            store.bound_target("sess-b:agent_7").unwrap(),
            Some("TAB-B".into())
        );
        assert_eq!(
            store.bound_target("../escape/attempt").unwrap(),
            Some("TAB-C".into())
        );
        store.bind("sess-a:main", "TAB-A2").unwrap();
        assert_eq!(
            store.bound_target("sess-a:main").unwrap(),
            Some("TAB-A2".into())
        );
        let files = std::fs::read_dir(&store.dir).unwrap().count();
        assert_eq!(
            files, 3,
            "each session has exactly one file inside the store"
        );
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn session_store_prefers_absolute_xdg_state_home() {
        let dir = session_store_dir(Some("/state".into()), Some("/home/u".into())).unwrap();
        assert_eq!(dir, PathBuf::from("/state/browser-cli/sessions"));
        let dir = session_store_dir(Some("relative".into()), Some("/home/u".into())).unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/home/u/.local/state/browser-cli/sessions")
        );
    }
}
