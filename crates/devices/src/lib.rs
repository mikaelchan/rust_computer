//! Memory-mapped device implementations used by the simulator.

pub mod ram;
pub mod rom;
pub mod simple_uart;

pub use ram::Ram;
pub use rom::Rom;
pub use simple_uart::SimpleUart;
