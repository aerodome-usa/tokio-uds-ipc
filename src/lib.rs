//! Generic UDS client/server implementation for IPC

mod client;
mod comm;
mod one_slot_channel;
mod server;
pub(crate) mod utils;

pub use client::borrowed as client_borrowed;
pub use client::owned as client_owned;
pub use client::{Client, RobustClient, RobustType, ServerType};
pub use comm::CodecError;
pub use server::{Error, IndependentHandlers, RequestStream, Server, independent_handlers, serve};
