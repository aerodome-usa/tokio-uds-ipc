//! Generic UDS client/server implementation for IPC

mod client;
mod comm;
mod server;
pub(crate) mod utils;

pub use client::{Client, RobustClient, RobustType, ServerType, SplitSinkOwned, SplitStreamOwned};
pub use comm::CodecError;
pub use server::{Error, IndependentHandlers, RequestStream, Server, independent_handlers, serve};
