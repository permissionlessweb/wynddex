use cosmwasm_std::{ConversionOverflowError, Decimal256, StdError, Uint256};
use thiserror::Error;

/// This enum describes factory contract errors
#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    ConversionOverflowError(#[from] ConversionOverflowError),

    #[error("Invalid value for trading start")]
    InvalidTradingStart {},

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Pair was already created")]
    PairWasCreated {},

    #[error("Pair was already registered")]
    PairWasRegistered {},

    #[error("Duplicate of pair configs")]
    PairConfigDuplicate {},

    #[error("Fee bps in pair config must be smaller than or equal to 10,000")]
    PairConfigInvalidFeeBps {},

    #[error("Pair config not found")]
    PairConfigNotFound {},

    #[error("Pair config disabled")]
    PairConfigDisabled {},

    #[error("Doubling assets in asset infos")]
    DoublingAssets {},

    #[error("Invalid referral commision: {0}")]
    InvalidReferralCommission(Decimal256),

    #[error("Can only init upgrade from cw-placeholder")]
    NotPlaceholder,

    #[error("Permissionless dex requires deposit to be set")]
    DepositNotSet {},

    #[error("Incorrect deposit: permissionless factory requires deposit as: {0}{1}")]
    DepositRequired(Uint256, String),

    #[error("Factory is in permissionless mode: deposit must be sent to create new pair")]
    PermissionlessRequiresDeposit {},
}

impl PartialEq for ContractError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            _ => core::mem::discriminant(self) == core::mem::discriminant(other),
        }
    }
}
