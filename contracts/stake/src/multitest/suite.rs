use std::collections::HashMap;

use anyhow::{bail, Error, Result as AnyResult};

use cosmwasm_std::{
    testing::MockApi, to_json_binary, Addr, Coin, Decimal256, Empty, StdResult, Uint128, Uint256,
};
use cw20::{BalanceResponse, Cw20Coin, Cw20ExecuteMsg, Cw20QueryMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as Cw20InstantiateMsg;
use cw_controllers::{Claim, ClaimsResponse};
use cw_multi_test::{App, AppResponse, Contract, ContractWrapper, Executor};
use wyndex::{
    asset::{AssetInfo, AssetInfoExt, AssetInfoValidated, AssetValidated},
    stake::{InstantiateMsg, UnbondingPeriod},
};

use crate::msg::{
    AllStakedResponse, AnnualizedReward, AnnualizedRewardsResponse, BondingInfoResponse,
    BondingPeriodInfo, DelegatedResponse, DistributedRewardsResponse, ExecuteMsg, QueryMsg,
    RewardsPowerResponse, StakedResponse, TotalStakedResponse, UnbondAllResponse,
    UndistributedRewardsResponse, WithdrawableRewardsResponse,
};
use wyndex::stake::{FundingInfo, ReceiveMsg};

pub const SEVEN_DAYS: u64 = 604800;

fn contract_stake() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new_with_empty(
        crate::contract::execute,
        crate::contract::instantiate,
        crate::contract::query,
    );

    Box::new(contract)
}

pub(super) fn contract_token() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new_with_empty(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    );

    Box::new(contract)
}

pub const JUNO_DENOM: &str = "juno";

pub(super) fn juno_power(amount: u128) -> Vec<(AssetInfoValidated, u128)> {
    vec![(AssetInfoValidated::Native(JUNO_DENOM.to_string()), amount)]
}

pub(super) fn juno(amount: u128) -> AssetValidated {
    AssetInfoValidated::Native(JUNO_DENOM.to_string()).with_balance(amount)
}

pub(super) fn native_token(denom: String, amount: u128) -> AssetValidated {
    AssetInfoValidated::Native(denom).with_balance(amount)
}

#[derive(Debug)]
pub struct SuiteBuilder {
    pub cw20_contract: Addr,
    pub tokens_per_power: Uint256,
    pub min_bond: Uint256,
    pub unbonding_periods: Vec<UnbondingPeriod>,
    pub admin: Option<Addr>,
    pub unbonder: Option<Addr>,
    pub initial_balances: Vec<Cw20Coin>,
    pub native_balances: Vec<(Addr, Coin)>,
}

impl SuiteBuilder {
    pub fn new() -> Self {
        Self {
            cw20_contract: Addr::unchecked(""),
            tokens_per_power: Uint256::from(1000u128),
            min_bond: Uint256::from(5000u128),
            unbonding_periods: vec![SEVEN_DAYS],
            admin: None,
            unbonder: None,
            initial_balances: vec![],
            native_balances: vec![],
        }
    }

    pub fn with_native_balances(mut self, denom: &str, balances: Vec<(&Addr, u128)>) -> Self {
        self.native_balances
            .extend(balances.into_iter().map(|(addr, amount)| {
                (
                    Addr::unchecked(addr),
                    Coin {
                        denom: denom.to_owned(),
                        amount: amount.into(),
                    },
                )
            }));
        self
    }

    pub fn with_initial_balances(mut self, balances: Vec<(&Addr, u128)>) -> Self {
        let initial_balances = balances
            .into_iter()
            .map(|(address, amount)| Cw20Coin {
                address: address.to_string(),
                amount: amount.into(),
            })
            .collect::<Vec<Cw20Coin>>();
        self.initial_balances = initial_balances;
        self
    }

    pub fn with_min_bond(mut self, min_bond: u128) -> Self {
        self.min_bond = Uint256::from(min_bond);
        self
    }

    pub fn with_tokens_per_power(mut self, tokens_per_power: u128) -> Self {
        self.tokens_per_power = Uint256::from(tokens_per_power);
        self
    }

    pub fn with_admin(mut self, admin: &Addr) -> Self {
        self.admin = Some(admin.to_owned());
        self
    }

    pub fn with_unbonder(mut self, unbonder: &Addr) -> Self {
        self.unbonder = Some(unbonder.to_owned());
        self
    }

    pub fn with_unbonding_periods(mut self, unbonding_periods: Vec<UnbondingPeriod>) -> Self {
        self.unbonding_periods = unbonding_periods;
        self
    }

    #[track_caller]
    pub fn build(self) -> Suite {
        let mut app: App = App::default();
        // provide initial native balances
        app.init_modules(|router, _, storage| {
            // group by address
            let mut balances = HashMap::<Addr, Vec<Coin>>::new();
            for (addr, coin) in self.native_balances {
                let addr_balance = balances.entry(addr).or_default();
                addr_balance.push(coin);
            }

            for (addr, coins) in balances {
                router
                    .bank
                    .init_balance(storage, &addr, coins)
                    .expect("init balance");
            }
        });

        let admin = MockApi::default().addr_make("admin");

        let token_id = app.store_code(contract_token());
        let token_contract = app
            .instantiate_contract(
                token_id,
                admin.clone(),
                &Cw20InstantiateMsg {
                    name: "vesting".to_owned(),
                    symbol: "VEST".to_owned(),
                    decimals: 9,
                    initial_balances: self.initial_balances,
                    mint: Some(MinterResponse {
                        minter: MockApi::default().addr_make("minter").to_string(),
                        cap: None,
                    }),
                    marketing: None,
                },
                &[],
                "vesting",
                None,
            )
            .unwrap();

        let stake_id = app.store_code(contract_stake());
        let stake_contract = app
            .instantiate_contract(
                stake_id,
                admin,
                &InstantiateMsg {
                    cw20_contract: token_contract.to_string(),
                    tokens_per_power: self.tokens_per_power,
                    min_bond: self.min_bond,
                    unbonding_periods: self.unbonding_periods,
                    admin: self.admin.map(|e| Some(e.to_string())).unwrap_or_default(),
                    unbonder: self
                        .unbonder
                        .map(|e| Some(e.to_string()))
                        .unwrap_or_default(),
                    max_distributions: 6,
                    converter: None,
                },
                &[],
                "stake",
                None,
            )
            .unwrap();

        Suite {
            app,
            token_id,
            stake_contract,
            token_contract,
        }
    }
}

pub struct Suite {
    pub app: App,
    token_id: u64,
    stake_contract: Addr,
    token_contract: Addr,
}

impl Suite {
    pub fn stake_contract(&self) -> Addr {
        self.stake_contract.clone()
    }

    pub fn token_contract(&self) -> String {
        self.token_contract.to_string()
    }

    // update block's time to simulate passage of time
    pub fn update_time(&mut self, time_update: u64) {
        let mut block = self.app.block_info();
        block.time = block.time.plus_seconds(time_update);
        self.app.set_block(block);
    }

    /// Create a new token contract and return the address
    pub fn instantiate_token(
        &mut self,
        owner: &Addr,
        token_name: &str,
        decimals: Option<u8>,
        balances: &[(&Addr, u128)],
    ) -> Addr {
        let init_msg = cw20_base::msg::InstantiateMsg {
            name: token_name.to_string(),
            symbol: token_name.to_string(),
            decimals: decimals.unwrap_or(6),
            initial_balances: balances
                .iter()
                .map(|(address, amount)| Cw20Coin {
                    address: address.to_string(),
                    amount: Uint128::from(*amount).into(),
                })
                .collect(),
            mint: Some(MinterResponse {
                minter: owner.to_string(),
                cap: None,
            }),
            marketing: None,
        };

        self.app
            .instantiate_contract(
                self.token_id,
                owner.clone(),
                &init_msg,
                &[],
                token_name,
                Some(owner.to_string()),
            )
            .unwrap()
    }

    fn unbonding_period_or_default(&self, unbonding_period: impl Into<Option<u64>>) -> u64 {
        // Use default SEVEN_DAYS unbonding period if none provided
        if let Some(up) = unbonding_period.into() {
            up
        } else {
            SEVEN_DAYS
        }
    }

    // create a new distribution flow for staking
    pub fn create_distribution_flow(
        &mut self,
        sender: &Addr,
        manager: &Addr,
        asset: AssetInfo,
        rewards: Vec<(UnbondingPeriod, Decimal256)>,
    ) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                sender.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::CreateDistributionFlow {
                    manager: manager.to_string(),
                    asset,
                    rewards,
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    // call to staking contract by sender
    pub fn delegate(
        &mut self,
        sender: &Addr,
        amount: u128,
        unbonding_period: impl Into<Option<u64>>,
    ) -> AnyResult<AppResponse> {
        self.delegate_as(sender, amount, unbonding_period, None)
    }

    // call to staking contract by sender
    pub fn delegate_as(
        &mut self,
        sender: &Addr,
        amount: u128,
        unbonding_period: impl Into<Option<u64>>,
        delegate_as: Option<&Addr>,
    ) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                sender.to_owned(),
                self.token_contract.clone(),
                &Cw20ExecuteMsg::Send {
                    contract: self.stake_contract.to_string(),
                    amount: amount.into(),
                    msg: to_json_binary(&ReceiveMsg::Delegate {
                        unbonding_period: self.unbonding_period_or_default(unbonding_period),
                        delegate_as: delegate_as.map(|s| s.to_string()),
                    })
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    // call to staking contract by sender
    pub fn mass_delegate(
        &mut self,
        sender: &Addr,
        amount: u128,
        unbonding_period: impl Into<Option<u64>>,
        delegate_to: &[(&Addr, u128)],
    ) -> AnyResult<AppResponse> {
        let delegate_to = delegate_to
            .iter()
            .map(|(a, b)| (a.to_string(), Uint256::from(*b)))
            .collect();

        Ok(self
            .app
            .execute_contract(
                sender.to_owned(),
                self.token_contract.clone(),
                &Cw20ExecuteMsg::Send {
                    contract: self.stake_contract.to_string(),
                    amount: amount.into(),
                    msg: to_json_binary(&ReceiveMsg::MassDelegate {
                        unbonding_period: self.unbonding_period_or_default(unbonding_period),
                        delegate_to,
                    })
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    // call to stake contract by sender
    pub fn rebond(
        &mut self,
        sender: &Addr,
        amount: u128,
        bond_from: impl Into<Option<u64>>,
        bond_to: impl Into<Option<u64>>,
    ) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                sender.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::Rebond {
                    tokens: amount.into(),
                    bond_from: self.unbonding_period_or_default(bond_from),
                    bond_to: self.unbonding_period_or_default(bond_to),
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn unbond(
        &mut self,
        sender: &Addr,
        amount: u128,
        unbonding_period: impl Into<Option<u64>>,
    ) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                sender.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::Unbond {
                    tokens: amount.into(),
                    unbonding_period: self.unbonding_period_or_default(unbonding_period),
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn quick_unbond(&mut self, sender: &Addr, stakers: &[&Addr]) -> AnyResult<AppResponse> {
        let stakers = stakers.iter().map(|s| s.to_string()).collect();
        Ok(self
            .app
            .execute_contract(
                sender.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::QuickUnbond { stakers },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn claim(&mut self, sender: &Addr) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                sender.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::Claim {},
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    // call to vesting contract
    pub fn transfer(
        &mut self,
        sender: &Addr,
        recipient: &str,
        amount: impl Into<Uint128>,
    ) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                sender.to_owned(),
                self.token_contract.clone(),
                &Cw20ExecuteMsg::Transfer {
                    recipient: recipient.into(),
                    amount: amount.into().into(),
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn distribute_funds<'s>(
        &mut self,
        executor: &Addr,
        sender: impl Into<Option<&'s Addr>>,
        funds: Option<AssetValidated>,
    ) -> AnyResult<AppResponse> {
        let sender = sender.into();

        if let Some(funds) = funds {
            let transfer_msg = funds
                .into_msg(self.stake_contract.clone())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            self.app
                .execute(Addr::unchecked(sender.unwrap_or(executor)), transfer_msg)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }

        Ok(self
            .app
            .execute_contract(
                executor.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::DistributeRewards {
                    sender: sender.map(Addr::to_string),
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn execute_fund_distribution<'s>(
        &mut self,
        executor: &Addr,
        sender: impl Into<Option<&'s str>>,
        funds: AssetValidated,
    ) -> AnyResult<AppResponse> {
        let _sender = sender.into();

        let curr_block = self.app.block_info().time;

        Ok(self
            .app
            .execute_contract(
                executor.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::FundDistribution {
                    funding_info: FundingInfo {
                        start_time: curr_block.seconds(),
                        distribution_duration: 100,
                        amount: funds.amount,
                    },
                },
                &[Coin {
                    denom: funds.info.to_string(),
                    amount: funds.amount,
                }],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn execute_fund_distribution_curve(
        &mut self,
        executor: &Addr,
        denom: impl Into<String>,
        amount: u128,
        distribution_duration: u64,
    ) -> AnyResult<AppResponse> {
        let curr_block = self.app.block_info().time;

        Ok(self
            .app
            .execute_contract(
                executor.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::FundDistribution {
                    funding_info: FundingInfo {
                        start_time: curr_block.seconds(),
                        distribution_duration,
                        amount: Uint256::from(amount),
                    },
                },
                &[Coin {
                    denom: denom.into(),
                    amount: Uint256::new(amount),
                }],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    // call to staking contract by sender
    pub fn execute_fund_distribution_with_cw20(
        &mut self,
        executor: &Addr,
        funds: AssetValidated,
    ) -> AnyResult<AppResponse> {
        let funds_amount = funds.amount;
        let curr_block = self.app.block_info().time;

        self.execute_fund_distribution_with_cw20_curve(
            executor,
            funds,
            FundingInfo {
                start_time: curr_block.seconds(),
                distribution_duration: 100,
                amount: funds_amount,
            },
        )
    }

    pub fn execute_fund_distribution_with_cw20_curve(
        &mut self,
        executor: &Addr,
        funds: AssetValidated,
        funding_info: FundingInfo,
    ) -> AnyResult<AppResponse> {
        let token = match funds.info {
            AssetInfoValidated::Token(contract_addr) => contract_addr,
            _ => bail!("Only tokens are supported for cw20 distribution"),
        };
        Ok(self
            .app
            .execute_contract(
                executor.to_owned(),
                token,
                &Cw20ExecuteMsg::Send {
                    contract: self.stake_contract.to_string(),
                    amount: funds.amount,
                    msg: to_json_binary(&ReceiveMsg::Fund { funding_info })
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn execute_unbond_all(&mut self, executor: &Addr) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                executor.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::UnbondAll {},
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn execute_stop_unbond_all(&mut self, executor: &Addr) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                executor.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::StopUnbondAll {},
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn withdraw_funds<'s>(
        &mut self,
        executor: &Addr,
        owner: impl Into<Option<&'s str>>,
        receiver: impl Into<Option<&'s str>>,
    ) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                executor.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::WithdrawRewards {
                    owner: match owner.into() {
                        Some(o) => Some(o.to_string()),
                        None => None,
                    },
                    receiver: match receiver.into() {
                        Some(r) => Some(r.to_string()),
                        None => None,
                    },
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    #[allow(dead_code)]
    pub fn delegate_withdrawal(
        &mut self,
        executor: &Addr,
        delegated: &str,
    ) -> AnyResult<AppResponse> {
        Ok(self
            .app
            .execute_contract(
                executor.to_owned(),
                self.stake_contract.clone(),
                &ExecuteMsg::DelegateWithdrawal {
                    delegated: delegated.to_owned(),
                },
                &[],
            )
            .map_err(|e| Error::msg(e.to_string()))?)
    }

    pub fn withdrawable_rewards(&self, owner: &Addr) -> StdResult<Vec<AssetValidated>> {
        let resp: WithdrawableRewardsResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::WithdrawableRewards {
                owner: owner.to_string(),
            },
        )?;
        Ok(resp.rewards)
    }

    pub fn distributed_funds(&self) -> StdResult<Vec<AssetValidated>> {
        let resp: DistributedRewardsResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::DistributedRewards {},
        )?;
        Ok(resp.distributed)
    }

    pub fn withdrawable_funds(&self) -> StdResult<Vec<AssetValidated>> {
        let resp: DistributedRewardsResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::DistributedRewards {},
        )?;
        Ok(resp.withdrawable)
    }

    pub fn undistributed_funds(&self) -> StdResult<Vec<AssetValidated>> {
        let resp: UndistributedRewardsResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::UndistributedRewards {},
        )?;
        Ok(resp.rewards)
    }

    #[allow(dead_code)]
    pub fn delegated(&self, owner: &Addr) -> StdResult<Addr> {
        let resp: DelegatedResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::Delegated {
                owner: owner.to_string(),
            },
        )?;
        Ok(resp.delegated)
    }

    /// returns address' balance of native token
    pub fn query_balance(&self, address: &Addr, denom: &str) -> StdResult<u128> {
        let resp = self.app.wrap().query_balance(address, denom)?;
        Ok(Uint128::try_from(resp.amount).unwrap().u128())
    }

    pub fn query_cw20_balance(&self, address: &Addr, cw20: impl Into<String>) -> StdResult<u128> {
        let balance: BalanceResponse = self.app.wrap().query_wasm_smart(
            cw20,
            &Cw20QueryMsg::Balance {
                address: address.to_string(),
            },
        )?;
        Ok(Uint128::try_from(balance.balance).unwrap().u128())
    }

    // returns address' balance on vesting contract
    pub fn query_balance_vesting_contract(&self, address: &Addr) -> StdResult<u128> {
        let balance: BalanceResponse = self.app.wrap().query_wasm_smart(
            self.token_contract.clone(),
            &Cw20QueryMsg::Balance {
                address: address.to_string(),
            },
        )?;
        Ok(Uint128::try_from(balance.balance).unwrap().u128())
    }

    // returns address' balance on vesting contract
    pub fn query_balance_staking_contract(&self) -> StdResult<u128> {
        let balance: BalanceResponse = self.app.wrap().query_wasm_smart(
            self.token_contract.clone(),
            &Cw20QueryMsg::Balance {
                address: self.stake_contract.to_string(),
            },
        )?;
        Ok(Uint128::try_from(balance.balance).unwrap().u128())
    }

    pub fn query_staked(
        &self,
        address: &Addr,
        unbonding_period: impl Into<Option<u64>>,
    ) -> StdResult<u128> {
        let staked: StakedResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::Staked {
                address: address.to_string(),
                unbonding_period: self.unbonding_period_or_default(unbonding_period),
            },
        )?;
        Ok(Uint128::try_from(staked.stake).unwrap().u128())
    }

    pub fn query_staked_periods(&self) -> StdResult<Vec<BondingPeriodInfo>> {
        let info: BondingInfoResponse = self
            .app
            .wrap()
            .query_wasm_smart(self.stake_contract.clone(), &QueryMsg::BondingInfo {})?;
        Ok(info.bonding)
    }

    pub fn query_all_staked(&self, address: &Addr) -> StdResult<AllStakedResponse> {
        let all_staked: AllStakedResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::AllStaked {
                address: address.to_string(),
            },
        )?;
        Ok(all_staked)
    }

    pub fn query_total_staked(&self) -> StdResult<u128> {
        let total_staked: TotalStakedResponse = self
            .app
            .wrap()
            .query_wasm_smart(self.stake_contract.clone(), &QueryMsg::TotalStaked {})?;
        Ok(Uint128::try_from(total_staked.total_staked).unwrap().u128())
    }

    pub fn query_claims(&self, address: &Addr) -> StdResult<Vec<Claim>> {
        let claims: ClaimsResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::Claims {
                address: address.to_string(),
            },
        )?;
        Ok(claims.claims)
    }

    pub fn query_annualized_rewards(
        &self,
    ) -> StdResult<Vec<(UnbondingPeriod, Vec<AnnualizedReward>)>> {
        let apr: AnnualizedRewardsResponse = self
            .app
            .wrap()
            .query_wasm_smart(self.stake_contract.clone(), &QueryMsg::AnnualizedRewards {})?;
        Ok(apr.rewards)
    }

    pub fn query_rewards_power(
        &self,
        address: &Addr,
    ) -> StdResult<Vec<(AssetInfoValidated, u128)>> {
        let rewards: RewardsPowerResponse = self.app.wrap().query_wasm_smart(
            self.stake_contract.clone(),
            &QueryMsg::RewardsPower {
                address: address.to_string(),
            },
        )?;

        Ok(rewards
            .rewards
            .into_iter()
            .map(|(a, p)| (a, Uint128::try_from(p).unwrap().u128()))
            .filter(|(_, p)| *p > 0)
            .collect())
    }

    pub fn query_total_rewards_power(&self) -> StdResult<Vec<(AssetInfoValidated, u128)>> {
        let rewards: RewardsPowerResponse = self
            .app
            .wrap()
            .query_wasm_smart(self.stake_contract.clone(), &QueryMsg::TotalRewardsPower {})?;

        Ok(rewards
            .rewards
            .into_iter()
            .map(|(a, p)| (a, Uint128::try_from(p).unwrap().u128()))
            .filter(|(_, p)| *p > 0)
            .collect())
    }

    pub fn query_unbond_all(&self) -> StdResult<bool> {
        let resp: UnbondAllResponse = self
            .app
            .wrap()
            .query_wasm_smart(self.stake_contract.clone(), &QueryMsg::UnbondAll {})?;

        Ok(resp.unbond_all)
    }
}
