//! DevTerm on GPUI. The window is `app`; product behavior lives in `logic`
//! and the runtime modules beside it.

pub mod app;
pub mod editor;
pub mod icons;
pub mod logic;
pub mod persist;
pub mod ssh_config;
pub mod ssh_session;
pub mod term_view;
pub mod theme;
pub mod vt;
