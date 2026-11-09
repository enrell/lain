//! The in-tree parsers (identify module, D-155): the release-name
//! tokenizer behind `lain.release.parse@1` and the three identifiers
//! behind `lain.media.identify@1` (comic, anime, generic, in that
//! binding order). They are the default providers; a component (the
//! lain-parser model, P4) can be bound before them without touching the
//! scan. Ports of `internal/plugins/{identify,release}`; the shared
//! parity fixture (`conformance/fixtures/parse-parity.json`) holds the
//! Go answers for every name the Go tests use.

pub mod identify;
pub mod release;
pub mod text;

use serde_json::{Value, json};

use lain_core::{Error, Provider};

pub use identify::{
    ANIME_ID, COMIC_ID, Candidate, GENERIC_ID, Proposal, identify_anime, identify_comic, identify_generic,
};
pub use release::{TOKENIZER_ID, tokenize};

pub const CAP_MEDIA_IDENTIFY: &str = "lain.media.identify@1";
pub const CAP_RELEASE_PARSE: &str = "lain.release.parse@1";

/// The longest release name `lain.release.parse@1` accepts.
pub const MAX_NAME: usize = 1024;

/// Which identifier a provider runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identifier {
    Comic,
    Anime,
    Generic,
}

impl Identifier {
    pub const ALL: [Identifier; 3] = [Identifier::Comic, Identifier::Anime, Identifier::Generic];

    fn id(self) -> &'static str {
        match self {
            Identifier::Comic => COMIC_ID,
            Identifier::Anime => ANIME_ID,
            Identifier::Generic => GENERIC_ID,
        }
    }

    /// The proposal; a decline is the zero proposal (not accepted).
    pub fn identify(self, c: &Candidate) -> Proposal {
        match self {
            Identifier::Comic => identify_comic(c).unwrap_or_default(),
            Identifier::Anime => identify_anime(c).unwrap_or_default(),
            Identifier::Generic => identify_generic(c),
        }
    }
}

impl Provider for Identifier {
    fn id(&self) -> &str {
        Identifier::id(*self)
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_MEDIA_IDENTIFY.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_MEDIA_IDENTIFY {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        if !op.is_empty() && op != "identify" {
            return Err(Error::invalid(format!("capability {cap} has no op {op}")));
        }
        let c: Candidate =
            serde_json::from_value(input).map_err(|e| Error::invalid(format!("Candidate required: {e}")))?;
        Ok(json!(self.identify(&c)))
    }
}

/// `lain-release-tokenizer`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tokenizer;

#[derive(serde::Deserialize)]
struct ParseInput {
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
}

impl Provider for Tokenizer {
    fn id(&self) -> &str {
        TOKENIZER_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_RELEASE_PARSE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_RELEASE_PARSE {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        if !op.is_empty() && op != "parse" {
            return Err(Error::invalid(format!("capability {cap} has no op {op}")));
        }
        let i: ParseInput =
            serde_json::from_value(input).map_err(|e| Error::invalid(format!("ReleaseParseInput required: {e}")))?;
        if i.name.trim().is_empty() || i.name.len() > MAX_NAME {
            return Err(Error::invalid("name must be non-empty text up to 1024 bytes"));
        }
        Ok(json!(tokenize(&i.name, &i.kind)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_check_their_input() {
        let a = Identifier::Anime;
        assert_eq!(a.invoke("lain.nope@1", "", json!({})).unwrap_err().code, "invalid-message");
        assert_eq!(a.invoke(CAP_MEDIA_IDENTIFY, "bogus", json!({})).unwrap_err().code, "invalid-message");
        assert_eq!(a.invoke(CAP_MEDIA_IDENTIFY, "identify", json!({"path": 42})).unwrap_err().code, "invalid-message");
        let out = a.invoke(CAP_MEDIA_IDENTIFY, "", json!({"path": "/m/notes.txt"})).unwrap();
        assert_eq!(out["title"], "", "declined");
        let out =
            Identifier::Generic.invoke(CAP_MEDIA_IDENTIFY, "identify", json!({"path": "/m/x/notes.txt"})).unwrap();
        assert_eq!(out["title"], "notes");
        let t = Tokenizer;
        assert_eq!(t.invoke(CAP_RELEASE_PARSE, "", json!({"name": " "})).unwrap_err().code, "invalid-message");
        assert_eq!(
            t.invoke(CAP_RELEASE_PARSE, "parse", json!({"name": "x".repeat(1025)})).unwrap_err().code,
            "invalid-message"
        );
        assert_eq!(t.invoke(CAP_RELEASE_PARSE, "parse", json!({"name": "Show S01E02"})).unwrap()["season"], 1);
    }
}
