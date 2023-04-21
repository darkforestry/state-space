use damms::amm::AMM;

pub struct StateChange {
    pub block_number: u64,
    pub state_change: Option<Vec<AMM>>,
}
