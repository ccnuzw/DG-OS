//! V1-FR-010 LAN collaboration (D039, D053, D088, D099, D101, D105, D107).
//!
//! - [`net`]: private-IPv4 interface choice and the `socket2` listener
//!   (no `SO_REUSEADDR`, D105), port validation (D107).
//! - [`core`]: in-memory access code, Cookie sessions, pairing limits,
//!   admission and revocation. Nothing here is persisted (D088).
//! - [`http`] and [`ws`]: the whitelisted routes of interface contract §14.
//!   They reach chat only through [`chat_port::ChatBackend`].
//! - [`chat_port`]: the FR-006 `ChatManager` adapter and the read-only
//!   target list.
//! - [`service`]: start/stop, preferred port and fallback, network watch,
//!   app exit; [`server`] is its accept loop.
//! - [`audit`]: the non-blocking audit sink over `LanAuditRepository`.

pub mod audit;
pub mod chat_port;
pub mod code;
pub mod core;
pub mod http;
pub mod net;
pub mod server;
pub mod service;
pub mod spa;
pub mod ws;

pub use audit::{AuditSink, DbAuditSink};
pub use chat_port::{ChatBackend, ChatHost, LanTargets, ManagedChat};
pub use core::{
    Clock, Issuer, LanCore, LanDevice, LanSessionStatus, RevokeReason, SystemClock,
    MAX_CHAT_SESSIONS, MAX_SESSIONS,
};
pub use http::{router, PeerAddr};
pub use net::{Network, SystemNetwork, DEFAULT_PORT};
pub use service::{LanConfig, LanIssuedCode, LanService, LanServiceState, LanStatus};
pub use spa::SpaFiles;
