use damms::amm::AMM;

pub struct StateChange {
    pub state_change: Option<Vec<AMM>>,
    pub block_number: u64,
}

impl StateChange {
    pub fn new(state_change: Option<Vec<AMM>>, block_number: u64) -> Self {
        Self {
            block_number,
            state_change,
        }
    }
}
