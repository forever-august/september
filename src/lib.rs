//! September - HTTP to NNTP Bridge Server
//!
//! A bridge server that provides HTTP access to NNTP newsgroups,
//! built with Leptos for the web interface and async-nntp for newsgroup connectivity.

pub mod app;
pub mod bridge;
pub mod error;
pub mod nntp;

pub use app::*;
pub use error::*;

#[cfg(test)]
mod tests {
    #[test]
    fn library_loads() {
        // Basic test to ensure library structure is valid
        assert!(true);
    }
}
