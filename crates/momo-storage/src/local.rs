//! SQLite repositories share one pool; cross-table commits remain single transactions.

use super::*;

mod characters;
mod connection;
mod conversations;
mod deleted;
mod deletion;
mod maintenance;
mod messages;
mod metadata;
mod responses;
mod reviews;
mod state;
