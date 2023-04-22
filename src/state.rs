use std::{
    collections::{HashMap, HashSet},
    sync::{mpsc::Receiver, Arc, RwLock},
};

use arraydeque::ArrayDeque;
use damms::amm::{AutomatedMarketMaker, AMM};
use ethers::{
    providers::{Middleware, PubsubClient},
    types::{Filter, H160, H256},
};

use crate::{
    error::{StateChangeError, StateSpaceError},
    state_change::StateChange,
};

//TODO:FIXME:
//mevmanager should take tobstrat closure and txpoolstrat closure, have some sort of logic maintaining the statespace changes
//One potential idea is to pass in the tx for the state changes stream or some stream, and then have the state being managed elsewhere not in the mev manager, not sure yet.
//also could be internal to the mev manager
pub type StateSpace = HashMap<H160, AMM>;

pub struct StateSpaceManager<M: Middleware, S: PubsubClient> {
    pub state: Arc<RwLock<StateSpace>>, //TODO: consider that the state should NEVER while routing is occurring, account for this
    pub last_synced_block: u64,
    pub state_change_cache: ArrayDeque<StateChange, 150>,
    pub middleware: Arc<M>,
    pub stream_middleware: Arc<S>,
}

impl<M, S> StateSpaceManager<M, S>
where
    M: Middleware,
    S: Middleware + PubsubClient,
{
    pub fn new(
        state: StateSpace,
        last_synced_block: u64,
        middleware: Arc<M>,
        stream_middleware: Arc<S>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(state)),
            last_synced_block,
            state_change_cache: ArrayDeque::new(),
            middleware,
            stream_middleware,
        }
    }

    pub fn get_block_filter(&self) -> Filter {
        //Create the event log signature
        let mut event_signatures: Vec<H256> = vec![];
        let mut amm_variants = HashSet::new();

        for amm in self.state.read().unwrap().values() {
            let variant = match amm {
                AMM::UniswapV2Pool(_) => 0,
                AMM::UniswapV3Pool(_) => 1,
                AMM::ERC4626Vault(_) => 2,
            };

            if !amm_variants.contains(&variant) {
                amm_variants.insert(variant);
                event_signatures.extend(amm.sync_on_event_signatures());
            }
        }

        //Create a new filter
        Filter::new().topic0(event_signatures)
    }

    //pub fn handle_state_changes(&mut self, logs: Vec<Log>){}
    //pub fn handle_state_change(&mut self, log: Log){}

    //listens to new blocks and handles state changes, sending an h256 block hash when a new block is produced
    //pub fn listen_for_new_blocks()-> Result<Receiver<H256>, StateSpaceError<M>> {}
    pub fn listen_for_new_blocks() -> Result<Receiver<H256>, StateSpaceError<M>> {
        let (tx, rx) = std::sync::mpsc::channel();

        tokio::spawn(async move {});

        //TODO: maybe collect handles to await

        //sub to new block headers

        //handle the state changes

        //send all of the affected amm address through the channel

        Ok(rx)
    }

    pub fn listen_for_state_changes(&self) -> Result<Receiver<Vec<H160>>, StateSpaceError<M>>
    where
        <S as Middleware>::Provider: PubsubClient,
    {
        let (tx, rx) = std::sync::mpsc::channel();

        let block_stream = self.stream_middleware.subscribe_blocks();
        tokio::spawn(async move {});

        //TODO: maybe collect handles to await

        //sub to new block headers

        //handle the state changes

        //send all of the affected amm address through the channel

        Ok(rx)
    }

    //update state from logs fn

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
