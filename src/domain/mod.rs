//! The request itself, independent of any window or protocol server: the form state, building and
//! sending a request, variables, the query string, redaction, and small helpers for what comes back.

pub mod model;
pub mod request;
pub mod http;
pub mod vars;
pub mod query;
pub mod redact;
pub mod outline;
pub mod filename;
pub mod timefmt;
