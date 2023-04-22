use std::sync::{PoisonError, RwLockWriteGuard};

use damms::errors::{ArithmeticError, DAMMError, EventLogError};

use ethers::prelude::{AbiError, ContractError};
use ethers::providers::spoof::State;
use ethers::providers::{Middleware, ProviderError};

use ethers::signers::WalletError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StateSpaceError<M>
where
    M: Middleware,
{
    #[error("Middleware error")]
    MiddlewareError(<M as Middleware>::Error),
    #[error("Provider error")]
    ProviderError(#[from] ProviderError),
    #[error("Contract error")]
    ContractError(#[from] ContractError<M>),
    #[error("ABI Codec error")]
    ABICodecError(#[from] AbiError),
    #[error("Eth ABI error")]
    EthABIError(#[from] ethers::abi::Error),
    #[error("CFMM error")]
    DAMMError(#[from] DAMMError<M>),
    #[error("Arithmetic error")]
    ArithmeticError(#[from] ArithmeticError),
    #[error("Wallet error")]
    WalletError(#[from] WalletError),
    #[error("Insufficient wallet funds for execution")]
    InsufficientWalletFunds(),
    #[error("Event log error")]
    EventLogError(#[from] EventLogError),
}

#[derive(Error, Debug)]
pub enum StateChangeError {
    #[error("No state changes in cache")]
    NoStateChangesInCache,
    #[error("Error when removing a state change from the front of the deque")]
    PopFrontError,
    #[error("State change cache capacity error")]
    CapacityError,
    #[error("Poisoned RWLock on AMM state")]
    PoisonedLockOnState,
}