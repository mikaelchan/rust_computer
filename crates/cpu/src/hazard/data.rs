/// Result of a basic RAW hazard check between adjacent instructions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataHazardStatus {
    pub stall: bool,
}

/// Detect a simple load-use style hazard between one producer and the next consumer.
#[must_use]
pub fn detect_raw_hazard(
    producer_rd: Option<u8>,
    consumer_rs1: Option<u8>,
    consumer_rs2: Option<u8>,
) -> DataHazardStatus {
    let Some(producer) = producer_rd else {
        return DataHazardStatus { stall: false };
    };

    let stall = producer != 0 && (consumer_rs1 == Some(producer) || consumer_rs2 == Some(producer));
    DataHazardStatus { stall }
}
