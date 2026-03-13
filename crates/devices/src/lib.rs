//! Memory-mapped device implementations used by the computer model.

pub mod interrupt_controller;
pub mod latency_adapter;
pub mod machine_software_interrupt;
pub mod machine_timer;
pub mod ram;
pub mod rom;
pub mod simple_uart;

pub use interrupt_controller::InterruptController;
pub use latency_adapter::LatencyAdapter;
pub use machine_software_interrupt::MachineSoftwareInterrupt;
pub use machine_timer::MachineTimer;
pub use ram::Ram;
pub use rom::Rom;
pub use simple_uart::SimpleUart;
