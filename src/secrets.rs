//! Opt-in "remember" for secret values (secret variables and the Bearer
//! token). They go into the operating system's credential store, never into
//! the state file or history database.

use crate::model::PersistedState;
use std::collections::{HashMap, HashSet};

pub const SERVICE: &str = "Plunger";
/// Windows Credential Manager caps a credential at 2560 bytes.
pub const MAX_SECRET_BYTES: usize = 2500;
const BEARER_KEY: &str = "bearer";

pub trait SecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// The real store: Windows Credential Manager / macOS Keychain. On other
/// platforms every call fails with a clear message rather than pretending to save.
pub struct OsStore {
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    service: String,
}

impl OsStore {
    pub fn new() -> Self {
        Self::with_service(SERVICE)
    }

    pub fn with_service(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
impl OsStore {
    fn entry(&self, key: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, key).map_err(|e| e.to_string())
    }
}

#[cfg(any(windows, target_os = "macos"))]
impl SecretStore for OsStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        match self.entry(key)?.get_secret() {
            Ok(bytes) => String::from_utf8(bytes).map(Some).map_err(|e| e.to_string()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.entry(key)?.set_secret(value.as_bytes()).map_err(|e| e.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
impl SecretStore for OsStore {
    fn get(&self, _key: &str) -> Result<Option<String>, String> {
        Err(UNSUPPORTED.to_string())
    }

    fn set(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }

    fn delete(&self, _key: &str) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
const UNSUPPORTED: &str = "Remembering secrets isn't supported on this platform yet.";

fn var_key(name: &str) -> String {
    format!("var:{}", name.trim())
}

/// Keeps the credential store in step with what the user has ticked, and
/// remembers what it has already written so nothing is rewritten needlessly.
#[derive(Default)]
pub struct SecretSync {
    /// Keys this app has stored, so ones no longer wanted can be deleted.
    known: HashSet<String>,
    /// Last value written per key, to skip identical rewrites.
    written: HashMap<String, String>,
    /// Keys that failed to load: never deleted until the user supplies a value,
    /// so a locked or failing store can't wipe good data.
    unreadable: HashSet<String>,
}

impl SecretSync {
    /// Fills in remembered values after startup. Returns human-readable problems.
    pub fn restore(&mut self, store: &dyn SecretStore, state: &mut PersistedState, bearer: &mut String) -> Vec<String> {
        let mut problems = Vec::new();
        let mut load = |sync: &mut SecretSync, key: String, target: &mut String| {
            sync.known.insert(key.clone());
            match store.get(&key) {
                Ok(Some(value)) => {
                    sync.written.insert(key, value.clone());
                    *target = value;
                }
                Ok(None) => {}
                Err(e) => {
                    sync.unreadable.insert(key);
                    problems.push(format!("Couldn't read a remembered secret: {e}"));
                }
            }
        };
        for v in state.variables.iter_mut().filter(|v| v.remember && v.is_secret() && !v.name.trim().is_empty()) {
            load(self, var_key(&v.name), &mut v.value);
        }
        if state.remember_bearer {
            load(self, BEARER_KEY.to_string(), bearer);
        }
        problems
    }

    /// Writes what should be remembered and deletes what no longer should be.
    pub fn persist(&mut self, store: &dyn SecretStore, state: &PersistedState, bearer: &str) -> Vec<String> {
        let mut problems = Vec::new();

        let mut desired: HashMap<String, &str> = HashMap::new();
        for v in &state.variables {
            if v.remember && v.is_secret() && !v.name.trim().is_empty() && !v.value.is_empty() {
                desired.insert(var_key(&v.name), &v.value);
            }
        }
        if state.remember_bearer && !bearer.is_empty() {
            desired.insert(BEARER_KEY.to_string(), bearer);
        }

        for (key, value) in &desired {
            if value.len() > MAX_SECRET_BYTES {
                problems.push(format!(
                    "A secret is too long to remember ({} bytes; the limit is {MAX_SECRET_BYTES}).",
                    value.len()
                ));
                continue;
            }
            if self.written.get(key).is_some_and(|w| w == value) {
                continue;
            }
            match store.set(key, value) {
                Ok(()) => {
                    self.written.insert(key.clone(), value.to_string());
                    self.known.insert(key.clone());
                    self.unreadable.remove(key);
                }
                Err(e) => problems.push(format!("Couldn't save a secret to the system credential store: {e}")),
            }
        }

        let stale: Vec<String> = self
            .known
            .iter()
            .filter(|k| !desired.contains_key(*k) && !self.unreadable.contains(*k))
            .cloned()
            .collect();
        for key in stale {
            match store.delete(&key) {
                Ok(()) => {
                    self.known.remove(&key);
                    self.written.remove(&key);
                }
                Err(e) => problems.push(format!("Couldn't remove a remembered secret: {e}")),
            }
        }
        problems
    }

    /// Un-ticks everything and removes every remembered secret.
    pub fn forget_all(&mut self, store: &dyn SecretStore, state: &mut PersistedState) -> Vec<String> {
        for v in &mut state.variables {
            v.remember = false;
        }
        state.remember_bearer = false;
        // The user asked for everything gone, including keys that failed to load.
        self.unreadable.clear();
        self.persist(store, state, "")
    }
}


/// An in-memory `SecretStore` for tests. Clones share the same data, so a
/// test can keep a handle while the app owns another.
#[cfg(test)]
pub(crate) mod test_support {
    use super::SecretStore;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    #[derive(Default, Clone)]
    pub struct MemoryStore {
        pub data: Rc<RefCell<HashMap<String, String>>>,
        pub writes: Rc<RefCell<usize>>,
        pub fail_reads: Rc<RefCell<bool>>,
    }

    impl SecretStore for MemoryStore {
        fn get(&self, key: &str) -> Result<Option<String>, String> {
            if *self.fail_reads.borrow() {
                return Err("store is locked".into());
            }
            Ok(self.data.borrow().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            *self.writes.borrow_mut() += 1;
            self.data.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            self.data.borrow_mut().remove(key);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryStore;
    use super::*;
    use crate::model::Variable;

    fn var(name: &str, value: &str, secret: bool, remember: bool) -> Variable {
        Variable { name: name.into(), value: value.into(), secret, remember }
    }

    fn state(vars: Vec<Variable>) -> PersistedState {
        PersistedState { variables: vars, ..Default::default() }
    }

    fn keys(store: &MemoryStore) -> Vec<String> {
        let mut k: Vec<String> = store.data.borrow().keys().cloned().collect();
        k.sort();
        k
    }

    #[test]
    fn a_remembered_secret_survives_a_restart_through_the_redacted_state() {
        let store = MemoryStore::default();
        let s = state(vec![var("apiKey", "SEKRET", true, true), var("host", "localhost", false, false)]);
        let mut sync = SecretSync::default();
        assert!(sync.persist(&store, &s, "").is_empty());
        assert_eq!(keys(&store), vec!["var:apiKey"]);

        // "restart": what is on disk has the value blanked but keeps the flags
        let mut reloaded = s.redacted();
        assert_eq!(reloaded.variables[0].value, "");
        let mut bearer = String::new();
        let problems = SecretSync::default().restore(&store, &mut reloaded, &mut bearer);
        assert!(problems.is_empty());
        assert_eq!(reloaded.variables[0].value, "SEKRET");
        assert_eq!(reloaded.variables[1].value, "localhost");
    }

    #[test]
    fn the_bearer_token_can_be_remembered_too() {
        let store = MemoryStore::default();
        let mut s = state(vec![]);
        s.remember_bearer = true;
        let mut sync = SecretSync::default();
        sync.persist(&store, &s, "tok123");
        assert_eq!(keys(&store), vec!["bearer"]);

        let mut bearer = String::new();
        SecretSync::default().restore(&store, &mut s.redacted(), &mut bearer);
        assert_eq!(bearer, "tok123");
    }

    #[test]
    fn nothing_is_stored_unless_ticked_and_secret() {
        let store = MemoryStore::default();
        let s = state(vec![
            var("plain", "v", false, true),      // remember ticked but not secret
            var("secretNoRemember", "v", true, false),
            var("emptyOne", "", true, true),     // nothing to remember
        ]);
        SecretSync::default().persist(&store, &s, "bearer-not-remembered");
        assert!(keys(&store).is_empty());
    }

    #[test]
    fn a_credential_looking_name_counts_as_secret_automatically() {
        let store = MemoryStore::default();
        let s = state(vec![var("authToken", "T", false, true)]);
        SecretSync::default().persist(&store, &s, "");
        assert_eq!(keys(&store), vec!["var:authToken"]);
    }

    #[test]
    fn unticking_removing_and_renaming_all_delete_the_old_entry() {
        let store = MemoryStore::default();
        let mut sync = SecretSync::default();
        let mut s = state(vec![var("a", "1", true, true), var("b", "2", true, true), var("c", "3", true, true)]);
        sync.persist(&store, &s, "");
        assert_eq!(keys(&store), vec!["var:a", "var:b", "var:c"]);

        s.variables[0].remember = false; // untick
        s.variables.remove(1); // delete the row
        s.variables[1].name = "renamed".into(); // rename c
        sync.persist(&store, &s, "");
        assert_eq!(keys(&store), vec!["var:renamed"]);
        assert_eq!(store.data.borrow()["var:renamed"], "3");
    }

    #[test]
    fn an_unchanged_value_is_not_rewritten_on_every_autosave() {
        let store = MemoryStore::default();
        let mut sync = SecretSync::default();
        let mut s = state(vec![var("k", "v1", true, true)]);
        sync.persist(&store, &s, "");
        sync.persist(&store, &s, "");
        sync.persist(&store, &s, "");
        assert_eq!(*store.writes.borrow(), 1);
        s.variables[0].value = "v2".into();
        sync.persist(&store, &s, "");
        assert_eq!(*store.writes.borrow(), 2);
        assert_eq!(store.data.borrow()["var:k"], "v2");
    }

    #[test]
    fn oversized_secrets_are_refused_with_a_message_not_silently_dropped() {
        let store = MemoryStore::default();
        let s = state(vec![var("big", &"x".repeat(MAX_SECRET_BYTES + 1), true, true)]);
        let problems = SecretSync::default().persist(&store, &s, "");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("too long"), "{problems:?}");
        assert!(keys(&store).is_empty());
    }

    #[test]
    fn a_store_that_cannot_be_read_never_gets_its_data_deleted() {
        let store = MemoryStore::default();
        store.data.borrow_mut().insert("var:k".into(), "precious".into());
        *store.fail_reads.borrow_mut() = true;

        let mut s = state(vec![var("k", "", true, true)]);
        let mut sync = SecretSync::default();
        let mut bearer = String::new();
        let problems = sync.restore(&store, &mut s, &mut bearer);
        assert_eq!(problems.len(), 1);
        assert_eq!(s.variables[0].value, "", "nothing was loaded");

        // autosave with the (blank) value must NOT wipe the stored secret
        sync.persist(&store, &s, "");
        assert_eq!(store.data.borrow()["var:k"], "precious");

        // once the user supplies a value it is saved normally
        s.variables[0].value = "new".into();
        sync.persist(&store, &s, "");
        assert_eq!(store.data.borrow()["var:k"], "new");
    }

    #[test]
    fn forget_all_removes_everything_and_unticks_the_boxes() {
        let store = MemoryStore::default();
        let mut sync = SecretSync::default();
        let mut s = state(vec![var("a", "1", true, true), var("b", "2", true, true)]);
        s.remember_bearer = true;
        sync.persist(&store, &s, "bt");
        assert_eq!(keys(&store).len(), 3);

        let problems = sync.forget_all(&store, &mut s);
        assert!(problems.is_empty());
        assert!(keys(&store).is_empty());
        assert!(s.variables.iter().all(|v| !v.remember) && !s.remember_bearer);
    }

    /// Talks to the real Windows Credential Manager under a throwaway service name.
    #[cfg(windows)]
    #[test]
    fn the_real_windows_credential_manager_round_trips() {
        let store = OsStore::with_service("Plunger (unit test)");
        let key = format!("roundtrip-{}", std::process::id());

        assert_eq!(store.get(&key).unwrap(), None);
        store.set(&key, "pässword ✓ with unicode").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("pässword ✓ with unicode"));
        store.set(&key, "changed").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("changed"));

        // the documented size limit really is the limit
        let biggest = "x".repeat(MAX_SECRET_BYTES);
        store.set(&key, &biggest).unwrap();
        assert_eq!(store.get(&key).unwrap().unwrap().len(), MAX_SECRET_BYTES);

        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        store.delete(&key).unwrap(); // deleting a missing entry is fine
    }
}
