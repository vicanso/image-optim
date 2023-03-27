#[macro_use]
pub mod macros;

pub use tracing::{error, info, warn};

pub fn clone_value_from_task_local<T>(value: &T) -> T
where
    T: Clone,
{
    value.clone()
}

tokio::task_local! {
    pub static TRACE_ID: String;
    pub static STARTED_AT: i64;
}
