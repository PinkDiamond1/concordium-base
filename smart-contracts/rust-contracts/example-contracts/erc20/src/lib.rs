#![cfg_attr(not(feature = "std"), no_std)]
use concordium_sc_base::{collections::*, *};

/*
 * An implementation of ERC-20 Token Standard used on the Ethereum network.
 * It provides standard functionality for transfering tokens and allowing
 * other accounts to transfer a certain amount from ones account.
 *
 * https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
 *
 * Instead of getter functions the information can be read directly from the
 * state. Events can be tracked in the log.
 */

// Types
type U999 = u64; // spec says u256 but we only have u64 at most

#[derive(SchemaType)]
struct InitParams {
    name:         String, // Name of the token
    symbol:       String, // Symbol of the token
    decimals:     u32,    // Number of decimals to show when displayed
    total_supply: U999,   // Total supply of tokens created
}

#[contract_state]
#[derive(Serialize, SchemaType)]
pub struct State {
    init_params: InitParams,
    balances:    BTreeMap<AccountAddress, U999>,
    // (owner, spender) => amount --- Owner allows spender to send the amount
    allowed: BTreeMap<(AccountAddress, AccountAddress), U999>,
}

#[derive(Serialize)]
enum Request {
    // (receive_account, amount) - Transfers 'amount' tokens from the sender account to
    // 'receive_account'
    TransferTo(AccountAddress, U999),
    // (owner_account, receive_account, amount) - Transfers 'amount' tokens from the
    // 'owner_account' to 'receive_account' if allowed
    TransferFromTo(AccountAddress, AccountAddress, U999),
    // (allowed_account, amount) - Allows 'allowed_account' to send up to 'amount' tokens from
    // the sender account. Called Approve in the erc20 spec. TODO: this request is insecure af
    // wrt tx ordering.
    AllowTransfer(AccountAddress, U999),
}

// Event printed in the log
#[derive(Serialize)]
enum Event {
    // `amount` of tokens are tranfered `from_account` to `to_account`
    // (from_account, to_account, amount)
    Transfer(AccountAddress, AccountAddress, U999),
    // `amount` of tokens are allowed to be tranfered by `spender_account` from `owner_account`
    // (owner_account, spender_account, amount)
    Approval(AccountAddress, AccountAddress, U999),
}

// Contract

#[init(name = "init")]
#[inline(always)]
fn contract_init<I: HasInitContext<()>, L: HasLogger>(
    ctx: &I,
    amount: Amount,
    logger: &mut L,
) -> InitResult<State> {
    ensure!(amount == 0, "The amount must be 0");

    let init_params: InitParams = ctx.parameter_cursor().get()?;

    // Let the creator have all the tokens
    let creator = ctx.init_origin();
    logger.log(&Event::Transfer(AccountAddress([0u8; 32]), creator, init_params.total_supply));
    let mut balances = BTreeMap::new();
    balances.insert(creator, init_params.total_supply);

    let state = State {
        init_params,
        balances,
        allowed: BTreeMap::new(),
    };
    Ok(state)
}

#[receive(name = "receive")]
#[inline(always)]
fn contract_receive<R: HasReceiveContext<()>, L: HasLogger, A: HasActions>(
    ctx: &R,
    receive_amount: Amount,
    logger: &mut L,
    state: &mut State,
) -> ReceiveResult<A> {
    ensure!(receive_amount == 0, "The amount must be 0");

    let msg: Request = ctx.parameter_cursor().get()?;

    let sender_address = match ctx.sender() {
        Address::Contract(_) => bail!("Only accounts can interact with this contract"),
        Address::Account(address) => address,
    };

    match msg {
        Request::TransferTo(receiver_address, amount) => {
            let sender_balance = *state.balances.get(&sender_address).unwrap_or(&0);
            ensure!(sender_balance >= amount, "Insufficient funds");

            let receiver_balance = *state.balances.get(&receiver_address).unwrap_or(&0);
            state.balances.insert(sender_address, sender_balance - amount);
            state.balances.insert(receiver_address, receiver_balance + amount);
            logger.log(&Event::Transfer(sender_address, receiver_address, amount));
        }
        Request::TransferFromTo(owner_address, receiver_address, amount) => {
            let allowed_amount = *state.allowed.get(&(owner_address, sender_address)).unwrap_or(&0);
            ensure!(
                allowed_amount >= amount,
                "The account owner is not allowing you to send this much"
            );

            let owner_balance = *state.balances.get(&owner_address).unwrap_or(&0);
            ensure!(owner_balance >= amount, "Insufficient funds");

            let receiver_balance = *state.balances.get(&receiver_address).unwrap_or(&0);
            state.allowed.insert((owner_address, sender_address), allowed_amount - amount);
            state.balances.insert(owner_address, owner_balance - amount);
            state.balances.insert(receiver_address, receiver_balance + amount);
            logger.log(&Event::Transfer(owner_address, receiver_address, amount));
        }
        Request::AllowTransfer(spender_address, amount) => {
            state.allowed.insert((sender_address, spender_address), amount);
            logger.log(&Event::Approval(sender_address, spender_address, amount));
        }
    }
    Ok(A::accept())
}

// (de)serialization

// Serializing the string by converting the string to a Vec of bytes, and use
// `serial` defined for Vec
fn serial_string<W: Write>(s: &str, out: &mut W) -> Result<(), W::Err> {
    let bytes = s.bytes().collect::<Vec<_>>();
    bytes.serial(out)
}
// Deserializing a string using deserial of Vec of bytes, and treat the byte
// vector as utf8 encoding
fn deserial_string<R: Read>(source: &mut R) -> Result<String, R::Err> {
    let bytes = Vec::deserial(source)?;
    let res = String::from_utf8(bytes).unwrap();
    Ok(res)
}

impl Serial for InitParams {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        serial_string(&self.name, out)?;
        serial_string(&self.symbol, out)?;
        self.decimals.serial(out)?;
        self.total_supply.serial(out)?;
        Ok(())
    }
}

impl Deserial for InitParams {
    fn deserial<R: Read>(source: &mut R) -> Result<Self, R::Err> {
        let name = deserial_string(source)?;
        let symbol = deserial_string(source)?;
        let decimals = u32::deserial(source)?;
        let total_supply = U999::deserial(source)?;
        Ok(InitParams {
            name,
            symbol,
            decimals,
            total_supply,
        })
    }
}

// Tests
#[cfg(test)]
pub mod tests {
    use super::*;
    use concordium_sc_base::test_infrastructure::*;

    #[test]
    /// Initialise token/contract giving the owner
    fn test_init() {
        // Setup context

        let init_origin = AccountAddress([1u8; 32]);

        let parameter = InitParams {
            name:         "USD".to_string(),
            symbol:       "$".to_string(),
            decimals:     0,
            total_supply: 100,
        };
        let parameter_bytes = to_bytes(&parameter);

        let mut ctx = InitContextTest::default();
        ctx.set_init_origin(init_origin);
        ctx.set_parameter(&parameter_bytes);

        // set up the logger so we can intercept and analyze them at the end.
        let mut logger = test_infrastructure::LogRecorder::init();

        // Execution
        let out = contract_init(&ctx, 0, &mut logger);

        // Tests
        match out {
            Err(_) => claim!(false, "Contract initialization failed."),
            Ok(state) => {
                claim_eq!(
                    state.allowed.len(),
                    0,
                    "No one is allowed to transfer from others account at this point"
                );
                claim_eq!(
                    *state.balances.get(&init_origin).unwrap(),
                    100,
                    "The creator of the contract/token should own all of the tokens"
                )
            }
        }
        // and make sure the correct logs were produced.
        claim_eq!(logger.logs.len(), 1, "Incorrect number of logs produced.");
        claim_eq!(
            logger.logs[0],
            to_bytes(&Event::Transfer(AccountAddress([0u8; 32]), init_origin, 100)),
            "Should log an initial transfer, when creating the token"
        );
    }

    #[test]
    /// Transfers tokens from the sender account
    fn test_receive_transfer_to() {
        // Setup context

        let from_account = AccountAddress([1u8; 32]);
        let to_account = AccountAddress([2u8; 32]);

        let parameter = Request::TransferTo(to_account, 70);
        let parameter_bytes = to_bytes(&parameter);

        let mut ctx = ReceiveContextTest::default();
        ctx.set_parameter(&parameter_bytes);
        ctx.set_sender(Address::Account(from_account));

        // Setup state
        let init_params = InitParams {
            name:         "USD".to_string(),
            symbol:       "$".to_string(),
            decimals:     0,
            total_supply: 100,
        };
        let mut balances = BTreeMap::new();
        balances.insert(from_account, 100);
        let allowed = BTreeMap::new();

        let mut state = State {
            init_params,
            balances,
            allowed,
        };
        let mut logger = test_infrastructure::LogRecorder::init();

        // Execution
        let res: ReceiveResult<test_infrastructure::ActionsTree> =
            contract_receive(&ctx, 0, &mut logger, &mut state);

        // Test
        let actions = match res {
            Err(_) => fail!("Contract receive support failed, but it should not have."),
            Ok(actions) => actions,
        };
        claim_eq!(
            actions,
            test_infrastructure::ActionsTree::accept(),
            "Transferring should result in an Accept action"
        );
        let from_balance = *state.balances.get(&from_account).unwrap();
        let to_balance = *state.balances.get(&to_account).unwrap();
        claim_eq!(
            from_balance,
            30,
            "The transferred amount should be subtracted from sender balance"
        );
        claim_eq!(to_balance, 70, "The transferred amount should be added to receiver balance");
        claim_eq!(logger.logs.len(), 1, "Incorrect number of logs produced.");
        claim_eq!(
            logger.logs[0],
            to_bytes(&Event::Transfer(from_account, to_account, 70)),
            "Should log the transfer"
        );
    }

    #[test]
    /// Sender transfer tokens between two other accounts
    ///
    /// - The amount is subtracted from the owners allowed funds
    /// - The transfer is successful
    fn test_receive_transfer_from_to() {
        // Setup context
        let spender_account = AccountAddress([1u8; 32]);
        let from_account = AccountAddress([2u8; 32]);
        let to_account = AccountAddress([3u8; 32]);

        let parameter = Request::TransferFromTo(from_account, to_account, 60);
        let parameter_bytes = to_bytes(&parameter);

        let mut ctx = ReceiveContextTest::default();
        ctx.set_parameter(&parameter_bytes);
        ctx.set_sender(Address::Account(spender_account));

        // Setup state
        let init_params = InitParams {
            name:         "Dollars".to_string(),
            symbol:       "$".to_string(),
            decimals:     0,
            total_supply: 200,
        };
        let mut balances = BTreeMap::new();
        balances.insert(from_account, 200);
        let mut allowed = BTreeMap::new();
        allowed.insert((from_account, spender_account), 100);

        let mut logger = test_infrastructure::LogRecorder::init();
        let mut state = State {
            init_params,
            balances,
            allowed,
        };

        // Execution
        let res: ReceiveResult<test_infrastructure::ActionsTree> =
            contract_receive(&ctx, 0, &mut logger, &mut state);

        // Test
        let actions = match res {
            Err(_) => fail!("Contract receive support failed, but it should not have."),
            Ok(actions) => actions,
        };
        claim_eq!(
            actions,
            test_infrastructure::ActionsTree::accept(),
            "Transferring should result in an Accept action"
        );
        let from_balance = *state.balances.get(&from_account).unwrap();
        let to_balance = *state.balances.get(&to_account).unwrap();
        let from_spender_allowed = *state.allowed.get(&(from_account, spender_account)).unwrap();
        claim_eq!(
            from_balance,
            140,
            "The transferred amount should be subtracted from sender balance"
        );
        claim_eq!(to_balance, 60, "The transferred amount should be added to receiver balance");
        claim_eq!(
            from_spender_allowed,
            40,
            "The transferred amount should be added to receiver balance"
        );

        claim_eq!(logger.logs.len(), 1, "Incorrect number of logs produced.");
        claim_eq!(
            logger.logs[0],
            to_bytes(&Event::Transfer(from_account, to_account, 60)),
            "Should log the transfer"
        );
    }

    #[test]
    /// Fail when attempting to transfer from account, without being allowed to
    /// the full amount
    fn test_receive_allow_transfer() {
        // Setup
        let spender_account = AccountAddress([1u8; 32]);
        let owner_account = AccountAddress([2u8; 32]);

        let parameter = Request::AllowTransfer(spender_account, 100);
        let parameter_bytes = to_bytes(&parameter);

        let mut ctx = ReceiveContextTest::default();
        ctx.set_parameter(&parameter_bytes);
        ctx.set_sender(Address::Account(owner_account));

        let init_params = InitParams {
            name:         "Dollars".to_string(),
            symbol:       "$".to_string(),
            decimals:     0,
            total_supply: 200,
        };
        let balances = BTreeMap::new();
        let allowed = BTreeMap::new();

        let mut logger = test_infrastructure::LogRecorder::init();
        let mut state = State {
            init_params,
            balances,
            allowed,
        };

        // Execution
        let res: ReceiveResult<test_infrastructure::ActionsTree> =
            contract_receive(&ctx, 0, &mut logger, &mut state);

        // Test
        let actions = match res {
            Ok(actions) => actions,
            Err(_) => fail!("The message is not expected to fail"),
        };
        claim_eq!(actions, test_infrastructure::ActionsTree::accept(), "Should accept the message");
        let owner_spender_allowed =
            *state.allowed.get(&(owner_account, spender_account)).unwrap_or(&0);
        claim_eq!(owner_spender_allowed, 100, "The allowed amount is not changed correctly");
        claim_eq!(logger.logs.len(), 1, "Incorrect number of logs produced.");
        claim_eq!(
            logger.logs[0],
            to_bytes(&Event::Approval(owner_account, spender_account, 100)),
            "Should log the approval"
        );
    }

    #[test]
    /// Fail when attempting to transfer from account, without being allowed to
    /// the full amount
    fn test_receive_transfer_to_not_allowed() {
        // Setup context
        let spender_account = AccountAddress([1u8; 32]);
        let from_account = AccountAddress([2u8; 32]);
        let to_account = AccountAddress([3u8; 32]);

        let parameter = Request::TransferFromTo(from_account, to_account, 110);
        let parameter_bytes = to_bytes(&parameter);

        let mut ctx = ReceiveContextTest::default();
        ctx.set_parameter(&parameter_bytes);
        ctx.set_sender(Address::Account(spender_account));

        // Setup state
        let init_params = InitParams {
            name:         "Dollars".to_string(),
            symbol:       "$".to_string(),
            decimals:     0,
            total_supply: 200,
        };
        let mut balances = BTreeMap::new();
        balances.insert(from_account, 200);
        let mut allowed = BTreeMap::new();
        allowed.insert((from_account, spender_account), 100);

        let mut state = State {
            init_params,
            balances,
            allowed,
        };
        let mut logger = test_infrastructure::LogRecorder::init();

        // Execution
        let res: ReceiveResult<test_infrastructure::ActionsTree> =
            contract_receive(&ctx, 0, &mut logger, &mut state);

        // Test
        claim!(res.is_err(), "The message is expected to fail");

        let from_balance = *state.balances.get(&from_account).unwrap_or(&0);
        let to_balance = *state.balances.get(&to_account).unwrap_or(&0);
        let from_spender_allowed = *state.allowed.get(&(from_account, spender_account)).unwrap();
        claim_eq!(from_balance, 200, "The balance of the owner account should be unchanged");
        claim_eq!(to_balance, 0, "The balance of the receiving account should be unchanged");
        claim_eq!(
            from_spender_allowed,
            100,
            "The allowed amount of the spender account should be unchanged"
        );
        claim_eq!(logger.logs.len(), 0, "Incorrect number of logs produced.");
    }

    #[test]
    /// Fail when attempting to transfer from account, without being allowed to
    /// the full amount
    fn test_receive_transfer_to_insufficient() {
        // Setup context
        let from_account = AccountAddress([2u8; 32]);
        let to_account = AccountAddress([3u8; 32]);

        let parameter = Request::TransferTo(to_account, 110);
        let parameter_bytes = to_bytes(&parameter);

        let mut ctx = ReceiveContextTest::default();
        ctx.set_parameter(&parameter_bytes);
        ctx.set_sender(Address::Account(from_account));

        // Setup state
        let init_params = InitParams {
            name:         "Dollars".to_string(),
            symbol:       "$".to_string(),
            decimals:     0,
            total_supply: 100,
        };
        let mut balances = BTreeMap::new();
        balances.insert(from_account, 100);
        let allowed = BTreeMap::new();

        let mut state = State {
            init_params,
            balances,
            allowed,
        };
        let mut logger = test_infrastructure::LogRecorder::init();

        // Execution
        let res: ReceiveResult<test_infrastructure::ActionsTree> =
            contract_receive(&ctx, 0, &mut logger, &mut state);

        // Test
        claim!(res.is_err(), "The message is expected to fail");
        let from_balance = *state.balances.get(&from_account).unwrap_or(&0);
        let to_balance = *state.balances.get(&to_account).unwrap_or(&0);
        claim_eq!(from_balance, 100, "The balance of the owner account should be unchanged");
        claim_eq!(to_balance, 0, "The balance of the receiving account should be unchanged");
        claim_eq!(logger.logs.len(), 0, "Incorrect number of logs produced.");
    }

    #[test]
    /// Fail when attempting to transfer from account with allowed amount, but
    /// insufficient funds
    fn test_receive_transfer_from_to_insufficient() {
        // Setup
        let from_account = AccountAddress([2u8; 32]);
        let to_account = AccountAddress([3u8; 32]);
        let spender_account = AccountAddress([4u8; 32]);

        let parameter = Request::TransferFromTo(from_account, to_account, 110);
        let parameter_bytes = to_bytes(&parameter);

        let mut ctx = ReceiveContextTest::default();
        ctx.set_parameter(&parameter_bytes);
        ctx.set_sender(Address::Account(spender_account));

        let init_params = InitParams {
            name:         "Dollars".to_string(),
            symbol:       "$".to_string(),
            decimals:     0,
            total_supply: 100,
        };
        let mut balances = BTreeMap::new();
        balances.insert(from_account, 100);
        let mut allowed = BTreeMap::new();
        allowed.insert((from_account, spender_account), 110);

        let mut logger = test_infrastructure::LogRecorder::init();
        let mut state = State {
            init_params,
            balances,
            allowed,
        };

        // Execution
        let res: ReceiveResult<test_infrastructure::ActionsTree> =
            contract_receive(&ctx, 0, &mut logger, &mut state);

        // Test
        claim!(res.is_err(), "The message is expected to fail");

        let from_balance = *state.balances.get(&from_account).unwrap_or(&0);
        let to_balance = *state.balances.get(&to_account).unwrap_or(&0);
        let from_spender_allowed =
            *state.allowed.get(&(from_account, spender_account)).unwrap_or(&0);
        claim_eq!(from_balance, 100, "The balance of the owner account should be unchanged");
        claim_eq!(to_balance, 0, "The balance of the receiving account should be unchanged");
        claim_eq!(
            from_spender_allowed,
            110,
            "The balance of the receiving account should be unchanged"
        );
        claim_eq!(logger.logs.len(), 0, "Incorrect number of logs produced.");
    }
}
