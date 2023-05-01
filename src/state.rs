use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU16, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
};

use arraydeque::ArrayDeque;
use damms::{
    amm::{AutomatedMarketMaker, AMM},
    errors::EventLogError,
};
use ethers::{
    providers::{Middleware, PubsubClient, StreamExt},
    types::{Filter, Log, H160, H256},
};
use tokio::{sync::mpsc::Receiver, task::JoinHandle};

use crate::error::{StateChangeError, StateSpaceError};

pub type StateSpace = HashMap<H160, AMM>;
pub type StateChangeCache = ArrayDeque<StateChange, 150>;

pub trait MiddlewarePubsub: Middleware {
    type PubsubProvider: 'static + PubsubClient;
}

impl<T> MiddlewarePubsub for T
where
    T: Middleware,
    T::Provider: 'static + PubsubClient,
{
    type PubsubProvider = T::Provider;
}

pub struct StateSpaceManager<M, S>
where
    M: 'static + Middleware,
    S: 'static + MiddlewarePubsub,
{
    pub state: Arc<RwLock<StateSpace>>,
    pub middleware: Arc<M>,
    pub stream_middleware: Arc<S>,
}

impl<M, S> StateSpaceManager<M, S>
where
    M: Middleware,
    S: MiddlewarePubsub,
{
    pub fn new(
        state: StateSpace,
        last_synced_block: u64,
        middleware: Arc<M>,
        stream_middleware: Arc<S>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(state)),
            middleware,
            stream_middleware,
        }
    }

    pub fn initialize_state(amms: Vec<AMM>) -> StateSpace {
        amms.into_iter()
            .map(|amm| (amm.address(), amm))
            .collect::<HashMap<H160, AMM>>()
    }

    pub fn get_block_filter(&self) -> Result<Filter, StateChangeError> {
        let mut event_signatures: Vec<H256> = vec![];
        let mut amm_variants = HashSet::new();

        for amm in self
            .state
            .read()
            .map_err(|_| StateChangeError::PoisonedLockOnState)?
            .values()
        {
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
        Ok(Filter::new().topic0(event_signatures))
    }

    //listens to new blocks and handles state changes, sending an h256 block hash when a new block is produced
    //pub fn listen_for_new_blocks()-> Result<Receiver<H256>, StateSpaceError<M>> {}
    pub async fn listen_for_new_blocks<'life0>(
        &self,
        mut last_synced_block: u64,
        channel_buffer: usize,
    ) -> Result<
        (
            Receiver<H256>,
            JoinHandle<Result<(), StateSpaceError<M, S>>>,
        ),
        StateSpaceError<M, S>,
    >
    where
        <S as Middleware>::Provider: PubsubClient,
    {
        let (tx, rx) = tokio::sync::mpsc::channel(channel_buffer);

        let state = self.state.clone();
        let mut state_change_cache: StateChangeCache = ArrayDeque::new();
        let middleware = self.middleware.clone();
        let stream_middleware: Arc<S> = self.stream_middleware.clone();
        let filter = self.get_block_filter()?;

        let handle: JoinHandle<Result<(), StateSpaceError<M, S>>> = tokio::spawn(async move {
            let mut block_stream = stream_middleware
                .subscribe_blocks()
                .await
                .map_err(StateSpaceError::PubsubClientError)?;

            while let Some(block) = block_stream.next().await {
                let chain_head_block_number = middleware
                    .get_block_number()
                    .await
                    .map_err(StateSpaceError::<M, S>::MiddlewareError)?
                    .as_u64();

                //If there is a reorg, unwind state changes from last_synced block to the chain head block number

                if chain_head_block_number <= last_synced_block {
                    unwind_state_changes(
                        state.clone(),
                        &mut state_change_cache,
                        chain_head_block_number,
                    )?;

                    //set the last synced block to the head block number
                    last_synced_block = chain_head_block_number;
                }

                let logs = middleware
                    .get_logs(
                        &filter
                            .clone()
                            .from_block(last_synced_block)
                            .to_block(chain_head_block_number),
                    )
                    .await
                    .map_err(StateSpaceError::MiddlewareError)?;

                if logs.is_empty() {
                    for block_number in last_synced_block..chain_head_block_number {
                        add_state_change_to_cache(
                            &mut state_change_cache,
                            StateChange::new(None, block_number),
                        )?;
                    }
                    last_synced_block = chain_head_block_number;
                } else {
                    last_synced_block = chain_head_block_number;
                    let amms_updated = handle_state_changes_from_logs(
                        state.clone(),
                        &mut state_change_cache,
                        logs,
                    )?;
                }

                if let Some(block_hash) = block.hash {
                    tx.send(block_hash).await?;
                } else {
                    return Err(StateSpaceError::BlockNumberNotFound);
                }
            }

            Ok::<(), StateSpaceError<M, S>>(())
        });

        Ok((rx, handle))
    }

    pub async fn listen_for_state_changes(
        &self,
        mut last_synced_block: u64,
        channel_buffer: usize,
    ) -> Result<
        (
            Receiver<Vec<H160>>,
            JoinHandle<Result<(), StateSpaceError<M, S>>>,
        ),
        StateSpaceError<M, S>,
    >
    where
        <S as Middleware>::Provider: PubsubClient,
    {
        let (tx, rx) = tokio::sync::mpsc::channel(channel_buffer);

        let state = self.state.clone();
        let mut state_change_cache: StateChangeCache = ArrayDeque::new();
        let middleware = self.middleware.clone();
        let stream_middleware: Arc<S> = self.stream_middleware.clone();
        let filter = self.get_block_filter()?;

        let handle: JoinHandle<Result<(), StateSpaceError<M, S>>> = tokio::spawn(async move {
            let mut block_stream = stream_middleware
                .subscribe_blocks()
                .await
                .map_err(StateSpaceError::PubsubClientError)?;

            while let Some(_block) = block_stream.next().await {
                let chain_head_block_number = middleware
                    .get_block_number()
                    .await
                    .map_err(StateSpaceError::<M, S>::MiddlewareError)?
                    .as_u64();

                //If there is a reorg, unwind state changes from last_synced block to the chain head block number

                if chain_head_block_number <= last_synced_block {
                    unwind_state_changes(
                        state.clone(),
                        &mut state_change_cache,
                        chain_head_block_number,
                    )?;

                    //set the last synced block to the head block number
                    last_synced_block = chain_head_block_number;
                }

                let logs = middleware
                    .get_logs(
                        &filter
                            .clone()
                            .from_block(last_synced_block)
                            .to_block(chain_head_block_number),
                    )
                    .await
                    .map_err(StateSpaceError::MiddlewareError)?;

                if logs.is_empty() {
                    for block_number in last_synced_block..chain_head_block_number {
                        add_state_change_to_cache(
                            &mut state_change_cache,
                            StateChange::new(None, block_number),
                        )?;
                    }
                    last_synced_block = chain_head_block_number;
                } else {
                    last_synced_block = chain_head_block_number;
                    let amms_updated = handle_state_changes_from_logs(
                        state.clone(),
                        &mut state_change_cache,
                        logs,
                    )?;
                    tx.send(amms_updated).await?;
                }
            }

            Ok::<(), StateSpaceError<M, S>>(())
        });

        Ok((rx, handle))
    }
}

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

//Unwinds the state changes cache for every block from the most recent state change cache back to the block to unwind -1
fn unwind_state_changes(
    state: Arc<RwLock<StateSpace>>,
    state_change_cache: &mut StateChangeCache,
    block_to_unwind: u64,
) -> Result<(), StateChangeError> {
    loop {
        //check if the most recent state change block is >= the block to unwind,
        if let Some(state_change) = state_change_cache.get(0) {
            if state_change.block_number >= block_to_unwind {
                if let Some(option_state_changes) = state_change_cache.pop_front() {
                    if let Some(state_changes) = option_state_changes.state_change {
                        for amm_state in state_changes {
                            state
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
    state_change_cache: &mut StateChangeCache,
    state_change: StateChange,
) -> Result<(), StateChangeError> {
    if state_change_cache.is_full() {
        state_change_cache.pop_back();
        state_change_cache
            .push_front(state_change)
            .map_err(|_| StateChangeError::CapacityError)?
    } else {
        state_change_cache
            .push_front(state_change)
            .map_err(|_| StateChangeError::CapacityError)?
    }
    Ok(())
}

pub fn handle_state_changes_from_logs(
    state: Arc<RwLock<StateSpace>>,
    state_change_cache: &mut StateChangeCache,
    logs: Vec<Log>,
) -> Result<Vec<H160>, StateChangeError> {
    let mut updated_amms_set = HashSet::new();
    let mut updated_amms = vec![];

    let mut last_log_block_number = if let Some(log) = logs.get(0) {
        get_block_number_from_log(log)?
    } else {
        return Ok(updated_amms);
    };

    let mut state_changes = vec![];

    let mut logs_iter = logs.into_iter();
    while let Some(log) = logs_iter.next() {
        let log_block_number = get_block_number_from_log(&log)?;

        //Commit state changes if the block has changed since last log
        if log_block_number != last_log_block_number {
            if state_changes.is_empty() {
                add_state_change_to_cache(
                    state_change_cache,
                    StateChange::new(None, last_log_block_number),
                )?;
            } else {
                add_state_change_to_cache(
                    state_change_cache,
                    StateChange::new(Some(state_changes), last_log_block_number),
                )?;
                state_changes = vec![];
            };

            last_log_block_number = log_block_number;
        }

        // check if the log is from an amm in the state space
        if let Some(amm) = state
            .write()
            .map_err(|_| StateChangeError::PoisonedLockOnState)?
            .get_mut(&log.address)
        {
            if !updated_amms_set.contains(&log.address) {
                updated_amms_set.insert(log.address);
                updated_amms.push(log.address);
            }

            state_changes.push(amm.clone());
            amm.sync_from_log(log)?;
        }
    }

    Ok(updated_amms)
}

pub fn get_block_number_from_log(log: &Log) -> Result<u64, EventLogError> {
    if let Some(block_number) = log.block_number {
        Ok(block_number.as_u64())
    } else {
        Err(damms::errors::EventLogError::LogBlockNumberNotFound)
    }
}
