//! Structured browser control for Chrome over CDP.
//!
//! The default is disposable isolation: [`ControlledBrowser::launch_isolated`]
//! starts headless Chrome in a temp profile that is destroyed on close.
//! Personal browsing ([`ControlledBrowser::launch_with_profile`] and
//! [`ControlledBrowser::attach`]) is explicit, opt-in, and documented as
//! acting with the user's full identity. Attached sessions never
//! terminate the user's browser process.
//!
//! The WebSocket layer is hand-rolled and dependency-free (loopback CDP
//! only); the crate depends on serde/serde_json alone.

pub mod browser;
pub mod profile;

pub(crate) mod cdp;
pub(crate) mod ws;

pub use browser::{BrowserLimits, ControlledBrowser, file_url};
pub use profile::{ProfileInfo, chrome_binary, list_profiles, user_data_dir};
