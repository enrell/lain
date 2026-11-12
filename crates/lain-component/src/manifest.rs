use serde::{Deserialize, Serialize};

/// How to run a component (`execution` in a manifest).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    pub kind: String,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// A component manifest (`<data-dir>/plugins/<id>.json`), the same
/// document the Go provisioner reads (`matrix.Manifest`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub execution: Execution,
    /// Optional, D-285: one plain sentence per capability this component
    /// brings, which `GET /api/plugins` shows when the server does not
    /// describe that capability itself.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub summaries: std::collections::BTreeMap<String, String>,
}

impl Manifest {
    /// The argv with `{sock}` and `{id}` filled in.
    pub fn argv(&self, sock: &str) -> Vec<String> {
        let mut argv = vec![self.execution.entrypoint.clone()];
        for a in &self.execution.args {
            argv.push(match a.as_str() {
                "{sock}" => sock.to_string(),
                "{id}" => self.id.clone(),
                other => other.to_string(),
            });
        }
        argv
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_manifest_parses_and_expands() {
        let m: Manifest = serde_json::from_str(
            r#"{"id":"community.anime-parser","version":"1.3.0","capabilities":["lain.media.identify@1"],
                "execution":{"kind":"process","entrypoint":"/bin/x","args":["--sock","{sock}","--id","{id}"]}}"#,
        )
        .unwrap();
        assert_eq!(m.argv("/run/a.sock"), vec!["/bin/x", "--sock", "/run/a.sock", "--id", "community.anime-parser"]);
    }
}
