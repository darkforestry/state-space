use std::{
    borrow::Borrow,
    collections::HashMap,
    sync::{Arc, RwLock},
};

use arraydeque::ArrayDeque;
use damms::amm::AMM;
use ethers::{providers::Middleware, types::H160};

pub type StateSpace = HashMap<H160, AMM>;

pub struct StateSpaceManager<M: Middleware> {
    pub state: Arc<RwLock<StateSpace>>,
    pub last_synced_block: u64,
    pub state_changes_cache: ArrayDeque<(u64, Option<Vec<AMM>>), 150>,
    pub middleware: Arc<M>,
}

impl<M> StateSpaceManager<M>
where
    M: Middleware,
{
    pub fn new(state: StateSpace, last_synced_block: u64, middleware: Arc<M>) -> Self {
        Self {
            state: Arc::new(RwLock::new(state)),
            last_synced_block,
            state_changes_cache: ArrayDeque::new(),
            middleware,
        }
    }

    //fn unwind_state_changes()

    //fn update_state_from_logs
}
