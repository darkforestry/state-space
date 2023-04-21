use std::{
    collections::HashMap,
    sync::{mpsc::Receiver, Arc, RwLock},
};

use arraydeque::ArrayDeque;
use damms::amm::{AutomatedMarketMaker, AMM};
use ethers::{prelude::k256::elliptic_curve::rand_core::block, providers::Middleware, types::H160};

use crate::{
    error::{StateChangeError, StateSpaceError},
    state_change::StateChange,
};

pub type StateSpace = HashMap<H160, AMM>;

pub struct StateSpaceManager<M: Middleware> {
    pub state: Arc<RwLock<StateSpace>>,
    pub last_synced_block: u64,
    pub state_change_cache: ArrayDeque<StateChange, 150>,
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
            state_change_cache: ArrayDeque::new(),
            middleware,
        }
    }

    //TODO: return an rx that will allow someone to subscribe and listen for the state changes

    //returns a reciever that signals when a specific amm has changed
    // pub fn listen_for_state_changes<M:Middleware>(ws provider) -> Result<Receiver<H160>, StateSpaceError<M>> {

    //     Ok(())
    // }

    //fn update_state_from_logs

    //Unwinds the state changes cache for every block from the most recent state change cache back to the block to unwind -1
    pub fn unwind_state_changes(&mut self, block_to_unwind: u64) -> Result<(), StateChangeError> {
        loop {
            //check if the most recent state change block is >= the block to unwind,
            if let Some(state_change) = self.state_change_cache.get(0) {
                if state_change.block_number >= block_to_unwind {
                    if let Some(option_state_changes) = self.state_change_cache.pop_front() {
                        if let Some(state_changes) = option_state_changes.state_change {
                            for amm_state in state_changes {
                                self.state
                                    .write()
                                    .map_err(|_| StateChangeError::PoisonedLockOnState)?
                                    .insert(amm_state.address(), amm_state);
                            }
                        }
                    } else {
                        //We know that there is a state change from state_change_cache.get(0) so when we pop front without returning a value, there is an issue
                        return Err(StateChangeError::PopFrontError);
                    }
                } else {
                    return Ok(());
                }
            } else {
                //We return an error here because we never want to be unwinding past where we have state changes.
                //For example, if you initialize a state space that syncs to block 100, then immediately after there is a chain reorg to 95, we can not roll back the state
                //changes for an accurate state space. In this case, we return an error
                return Err(StateChangeError::NoStateChangesInCache);
            }
        }
    }

    pub fn add_state_change_to_cache(
        &mut self,
        state_change: StateChange,
    ) -> Result<(), StateChangeError> {
        if self.state_change_cache.is_full() {
            self.state_change_cache.pop_back();
            self.state_change_cache
                .push_front(state_change)
                .map_err(|_| StateChangeError::CapacityError)?
        } else {
            self.state_change_cache
                .push_front(state_change)
                .map_err(|_| StateChangeError::CapacityError)?
        }
        Ok(())
    }
}
