//! Shipment tracking (scope Section 12): a carrier-provider abstraction and the poll
//! engine that writes normalized `shipment_event` rows and stops on delivery. Shared by
//! the API (`POST /shipments/{id}/poll`) and the standalone poller binary.

pub mod carrier;
pub mod poll;
pub mod trackingmore;

pub use carrier::{provider_from_env, CarrierProvider, CarrierUpdate, MockProvider, NoneProvider};
pub use poll::{poll_active_shipments, poll_shipment, PollOutcome};
pub use trackingmore::TrackingMoreProvider;
