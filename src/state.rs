use std::{
    collections::{HashMap, HashSet},
    sync::{mpsc::Receiver, Arc, RwLock},
};

use arraydeque::ArrayDeque;
use damms::{
    amm::{self, AutomatedMarketMaker, AMM},
    errors::EventLogError,
};
use ethers::{
    providers::{Middleware, PubsubClient, StreamExt},
    types::{Filter, Log, H160, H256},
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
    state_change_cache: ArrayDeque<StateChange, 150>,
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

    pub fn handle_state_changes_from_logs(
        &mut self,
        logs: &[Log],
    ) -> Result<Vec<H160>, StateChangeError> {
        let mut amms_changed = vec![];

        let mut last_log_block_number = if let Some(log) = logs.get(0) {
            self.get_block_number_from_log(log)?
        } else {
            return Ok(amms_changed);
        };

        let mut state_changes = vec![];

        while let Some(log) = logs[1..].iter().next() {
            let log_block_number = self.get_block_number_from_log(log)?;

            //Commit state chnages if the block has changed since last log
            if log_block_number != last_log_block_number {
                if state_changes.is_empty() {
                    self.add_state_change_to_cache(StateChange::new(None, last_log_block_number));
                } else {
                    self.add_state_change_to_cache(StateChange::new(
                        Some(state_changes),
                        last_log_block_number,
                    ));
                };

                state_changes = vec![];
            }

            // check if the log is from an amm in the state space
            if let Some(amm) = self
                .state
                .write()
                .map_err(|_| StateChangeError::PoisonedLockOnState)?
                .get_mut(&log.address)
            {
                state_changes.push(amm.clone());
                amm.sync_from_log(&log);
            }
        }

        Ok(amms_changed)
    }

    pub fn get_block_number_from_log(&self, log: &Log) -> Result<u64, EventLogError> {
        if let Some(block_number) = log.block_number {
            Ok(block_number.as_u64())
        } else {
            return Err(damms::errors::EventLogError::LogBlockNumberNotFound);
        }
    }
    pub fn handle_state_change_from_log(&mut self, log: Log) {}

    //listens to new blocks and handles state changes, sending an h256 block hash when a new block is produced
    //pub fn listen_for_new_blocks()-> Result<Receiver<H256>, StateSpaceError<M>> {}
    pub fn listen_for_new_blocks() -> Result<Receiver<H256>, StateSpaceError<M>> {
        //TODO: have some check here to make sure we arent updating state from other functions

        let (tx, rx) = std::sync::mpsc::channel();

        tokio::spawn(async move {});

        //TODO: maybe collect handles to await

        //sub to new block headers

        //handle the state changes

        //send all of the affected amm address through the channel

        Ok(rx)
    }

    pub async fn listen_for_state_changes(
        &'static mut self,
    ) -> Result<Receiver<Vec<H160>>, StateSpaceError<M>>
    where
        <S as Middleware>::Provider: 'static + PubsubClient,
    {
        //TODO: have some check here to make sure we arent updating state from other functions

        let (tx, rx) = std::sync::mpsc::channel();

        tokio::spawn(async move {
            let mut block_stream = self
                .stream_middleware
                .subscribe_blocks()
                .await
                .expect("handle this error TODO");

            let filter = self.get_block_filter();

            while let Some(_block) = block_stream.next().await {
                //TODO: we could potentially remove this but have it for now because we were encountering issues with not setting the "to" block
                let chain_head_block_number = self
                    .middleware
                    .get_block_number()
                    .await
                    .map_err(StateSpaceError::MiddlewareError)?
                    .as_u64();

                //If there is a reorg, unwind state changes from last_synced block to the chain head block number
                if chain_head_block_number <= self.last_synced_block {
                    self.unwind_state_changes(chain_head_block_number)?;

                    //set the last synced block to the head block number
                    self.last_synced_block = chain_head_block_number;
                }

                let logs = self
                    .middleware
                    .get_logs(
                        &filter
                            .clone()
                            .from_block(self.last_synced_block)
                            .to_block(chain_head_block_number),
                    )
                    .await
                    .expect("TODO: Need to handle this error");

                if logs.is_empty() {
                    for block_number in self.last_synced_block..chain_head_block_number {
                        self.add_state_change_to_cache(StateChange::new(block_number, None));
                    }

                    self.last_synced_block = chain_head_block_number;
                } else {
                    self.last_synced_block = chain_head_block_number;

                    self.handle_state_changes_from_logs(&logs);
                }
            }

            Ok(())
        });

        //TODO: maybe collect handles to await

        //sub to new block headers

        //handle the state changes

        //send all of the affected amm address through the channel

        Ok(rx)
    }

    //update state from logs fn

    //Unwinds the state changes cache for every block from the most recent state change cache back to the block to unwind -1
    fn unwind_state_changes(&mut self, block_to_unwind: u64) -> Result<(), StateChangeError> {
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

    fn add_state_change_to_cache(
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
