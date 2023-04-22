use damms::amm::AMM;

pub struct StateChange {
    pub block_number: u64,
    pub state_change: Option<Vec<AMM>>,
}

impl StateChange {
    pub fn new(block_number: u64, state_change: Option<Vec<AMM>>) -> Self {
        Self {
            block_number,
            state_change,
        }
    }
}
