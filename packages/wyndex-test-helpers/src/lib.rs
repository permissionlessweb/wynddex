use std::collections::HashMap;
use std::ops::Index;

use cosmwasm_std::Addr;
use cosmwasm_std::testing::MockApi;

/// Standard named test accounts with properly-generated bech32 addresses.
///
/// All addresses are derived via [`MockApi::addr_make`], which produces real
/// bech32-encoded addresses rather than raw string placeholders. This means
/// address validation in contracts will behave the same as on-chain.
///
/// ## Named fields (backwards-compatible)
/// The six well-known roles (`owner`, `user`, `whale`, `trader`,
/// `beneficiary`, `fee_receiver`) are also accessible as `accounts[0]`
/// through `accounts[5]`.
///
/// ## Infinite accounts
/// Call `accounts.addr("any_label")` to get (or lazily create) a
/// deterministic bech32 address for any label.  The same label always
/// returns the same address within a process.
///
/// ```ignore
/// let mut a = TestAccounts::new(app.api());
/// let pair   = a.addr("pair0000");   // new label, created on first call
/// let pair2  = a.addr("pair0000");   // same addr returned
/// let first  = a[0];                 // owner
/// let owner  = a["owner"];           // same as a.owner
/// ```
#[derive(Clone)]
pub struct TestAccounts {
    // Well-known roles – kept as public fields for ergonomic destructuring.
    pub owner: Addr,
    pub user: Addr,
    pub whale: Addr,
    pub trader: Addr,
    pub beneficiary: Addr,
    pub fee_receiver: Addr,

    /// Ordered list – `self[n]` indexes into this.
    /// Slot 0-5 mirror the six named fields above.
    list: Vec<Addr>,
    /// O(1) lookup by label string.
    map: HashMap<String, usize>,
    /// Kept so callers can mint new addresses after construction.
    api: MockApi,
}

impl TestAccounts {
    /// Build accounts from a specific [`MockApi`] instance.
    ///
    /// Pass `app.api()` in multi-test suites to keep all addresses consistent
    /// with contracts instantiated inside the same `App`.
    pub fn new(api: &MockApi) -> Self {
        let names = ["owner", "user", "whale", "trader", "beneficiary", "fee_receiver"];
        let addrs: Vec<Addr> = names.iter().map(|n| api.addr_make(n)).collect();

        let mut map = HashMap::new();
        for (i, name) in names.iter().enumerate() {
            map.insert(name.to_string(), i);
        }

        Self {
            owner: addrs[0].clone(),
            user: addrs[1].clone(),
            whale: addrs[2].clone(),
            trader: addrs[3].clone(),
            beneficiary: addrs[4].clone(),
            fee_receiver: addrs[5].clone(),
            list: addrs,
            map,
            api: *api,
        }
    }

    /// Return the address for `label`, creating it if it hasn't been seen before.
    ///
    /// The address is generated deterministically via `MockApi::addr_make(label)`.
    pub fn addr(&mut self, label: &str) -> Addr {
        if let Some(&idx) = self.map.get(label) {
            return self.list[idx].clone();
        }
        let addr = self.api.addr_make(label);
        let idx = self.list.len();
        self.list.push(addr.clone());
        self.map.insert(label.to_string(), idx);
        addr
    }

    /// Total number of accounts (named + any extras created via `addr`).
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// Index by position: `accounts[0]` == `accounts.owner`.
impl Index<usize> for TestAccounts {
    type Output = Addr;
    fn index(&self, idx: usize) -> &Addr {
        &self.list[idx]
    }
}

/// Index by label: `accounts["owner"]` == `accounts.owner`.
/// Panics if the label has not been created yet – use `addr()` to lazily create.
impl Index<&str> for TestAccounts {
    type Output = Addr;
    fn index(&self, label: &str) -> &Addr {
        let idx = self.map[label];
        &self.list[idx]
    }
}

impl Default for TestAccounts {
    /// Build accounts using [`MockApi::default()`].
    ///
    /// Suitable for pure unit tests that don't have a live `App`.
    fn default() -> Self {
        Self::new(&MockApi::default())
    }
}
