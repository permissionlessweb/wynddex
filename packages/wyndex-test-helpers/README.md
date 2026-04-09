# wyndex-test-helpers

Reusable test accounts for Wyndex contract tests.

## Why this exists

CosmWasm v3 enforces proper bech32 address validation. Tests that use
`owner` bypass this validation entirely — the string
`"owner"` is not a valid bech32 address and would be rejected on-chain.

`TestAccounts` uses `MockApi::addr_make` to generate real, deterministic
bech32 addresses from human-readable names. This means:

- Address validation inside contracts behaves the same as on-chain.
- Bugs where a contract rejects an invalid address are caught in tests,
  not discovered in production.
- Account names stay readable in test output (`"owner"`, `"whale"`, etc.)
  while the underlying `Addr` is a proper encoded address.

## Usage

### Multi-test suites (with `App`)

Derive accounts from the app's own API so all addresses are consistent:

```rust
use wyndex_test_helpers::TestAccounts;

let mut app = App::default();
let accounts = TestAccounts::new(app.api());

// Use in instantiate, execute, etc.
app.instantiate_contract(code_id, accounts.owner.clone(), &msg, &[], "label", None);
```

### Unit tests (without `App`)

Use the `Default` impl, which internally uses `MockApi::default()`:

```rust
use wyndex_test_helpers::TestAccounts;

let accounts = TestAccounts::default();
let info = message_info(&accounts.owner, &[]);
```

## Available accounts

| Field          | Human name      | Purpose                              |
|----------------|-----------------|--------------------------------------|
| `owner`        | `"owner"`       | Contract owner / admin               |
| `user`         | `"user"`        | Generic user / sender                |
| `whale`        | `"whale"`       | Large liquidity provider             |
| `trader`       | `"trader"`      | Nominated trader                     |
| `beneficiary`  | `"beneficiary"` | Fee or reward recipient              |
| `fee_receiver` | `"fee_receiver"`| Protocol fee collection address      |

Need a one-off address not in the list? Use `app.api().addr_make("my_name")`
directly — you don't need to add it here unless multiple test files share it.
