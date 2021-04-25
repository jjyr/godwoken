use crate::sudt::build_l2_sudt_script;
use crate::{
    error::{AccountError, DepositionError, Error, WithdrawalError},
    RollupContext,
};
use gw_common::{builtins::CKB_SUDT_ACCOUNT_ID, state::State, CKB_SUDT_SCRIPT_ARGS};
use gw_traits::CodeStore;
use gw_types::{
    bytes::Bytes,
    core::ScriptHashType,
    offchain::RunResult,
    packed::{DepositionRequest, Script, WithdrawalRequest},
    prelude::*,
};

pub trait StateExt {
    fn create_account_from_script(&mut self, script: Script) -> Result<u32, Error>;
    fn apply_run_result(&mut self, run_result: &RunResult) -> Result<(), Error>;
    fn apply_deposition_request(
        &mut self,
        ctx: &RollupContext,
        deposition_request: &DepositionRequest,
    ) -> Result<(), Error>;

    fn apply_withdrawal_request(
        &mut self,
        ctx: &RollupContext,
        withdrawal_request: &WithdrawalRequest,
    ) -> Result<(), Error>;

    fn apply_deposition_requests(
        &mut self,
        ctx: &RollupContext,
        deposition_requests: &[DepositionRequest],
    ) -> Result<(), Error> {
        for request in deposition_requests {
            self.apply_deposition_request(ctx, request)?;
        }
        Ok(())
    }

    fn apply_withdrawal_requests(
        &mut self,
        ctx: &RollupContext,
        withdrawal_requests: &[WithdrawalRequest],
    ) -> Result<(), Error> {
        for request in withdrawal_requests {
            self.apply_withdrawal_request(ctx, request)?;
        }

        Ok(())
    }
}

impl<S: State + CodeStore> StateExt for S {
    fn create_account_from_script(&mut self, script: Script) -> Result<u32, Error> {
        // Godwoken requires account's script using ScriptHashType::Type
        if script.hash_type() != ScriptHashType::Type.into() {
            return Err(AccountError::UnknownScript.into());
        }
        let script_hash = script.hash();
        self.insert_script(script_hash.into(), script);
        let id = self.create_account(script_hash.into())?;
        Ok(id)
    }

    fn apply_run_result(&mut self, run_result: &RunResult) -> Result<(), Error> {
        for (k, v) in &run_result.write_values {
            self.update_raw(*k, *v)?;
        }
        if let Some(id) = run_result.account_count {
            self.set_account_count(id)?;
        }
        for (script_hash, script) in &run_result.new_scripts {
            self.insert_script(*script_hash, Script::from_slice(&script).expect("script"));
        }
        for (data_hash, data) in &run_result.write_data {
            // register data hash into SMT
            self.store_data_hash(*data_hash)?;
            self.insert_data(*data_hash, Bytes::from(data.clone()));
        }
        Ok(())
    }

    fn apply_deposition_request(
        &mut self,
        ctx: &RollupContext,
        request: &DepositionRequest,
    ) -> Result<(), Error> {
        let script = request.script();
        {
            if script.hash_type() != ScriptHashType::Type.into() {
                eprintln!("Invalid deposit: unexpected hash_type: Data");
                return Err(Error::Deposition(DepositionError::DepositUnknownEoALock));
            }
            if ctx
                .rollup_config
                .allowed_eoa_type_hashes()
                .into_iter()
                .all(|type_hash| script.code_hash() != type_hash)
            {
                eprintln!(
                    "Invalid deposit: unknown code_hash: {:?}",
                    hex::encode(script.code_hash().as_slice())
                );
                return Err(Error::Deposition(DepositionError::DepositUnknownEoALock));
            }
            let args: Bytes = script.args().unpack();
            if args.len() < 52 {
                eprintln!(
                    "Invalid deposit: expect rollup_type_hash in the args but args is too short, len: {}",
                    args.len()
                );
                return Err(Error::Deposition(DepositionError::DepositUnknownEoALock));
            }
            if &args[..32] != ctx.rollup_script_hash.as_slice() {
                eprintln!(
                    "Invalid deposit: rollup_type_hash mismatch, rollup_script_hash: {}, args[..32]: {}",
                    hex::encode(ctx.rollup_script_hash.as_slice()),
                    hex::encode(&args[..32]),
                );
                return Err(Error::Deposition(DepositionError::DepositUnknownEoALock));
            }
        }

        // find or create user account
        let account_script_hash = request.script().hash();
        let id = match self.get_account_id_by_script_hash(&account_script_hash.into())? {
            Some(id) => id,
            None => {
                self.insert_script(account_script_hash.into(), request.script());
                self.create_account(account_script_hash.into())?
            }
        };
        // mint CKB
        let capacity: u64 = request.capacity().unpack();
        self.mint_sudt(CKB_SUDT_ACCOUNT_ID, id, capacity.into())?;
        let sudt_script_hash = request.sudt_script_hash().unpack();
        let amount = request.amount().unpack();
        if sudt_script_hash != CKB_SUDT_SCRIPT_ARGS.into() {
            // find or create Simple UDT account
            let l2_sudt_script = build_l2_sudt_script(&ctx, &sudt_script_hash);
            let l2_sudt_script_hash: [u8; 32] = l2_sudt_script.hash();
            let sudt_id = match self.get_account_id_by_script_hash(&l2_sudt_script_hash.into())? {
                Some(id) => id,
                None => {
                    self.insert_script(l2_sudt_script_hash.into(), l2_sudt_script);
                    self.create_account(l2_sudt_script_hash.into())?
                }
            };
            // prevent fake CKB SUDT, the caller should filter these invalid depositions
            if sudt_id == CKB_SUDT_ACCOUNT_ID {
                return Err(AccountError::InvalidSUDTOperation.into());
            }
            // mint SUDT
            self.mint_sudt(sudt_id, id, amount)?;
        } else if amount != 0 {
            return Err(DepositionError::DepositFakedCKB.into());
        }

        Ok(())
    }

    fn apply_withdrawal_request(
        &mut self,
        ctx: &RollupContext,
        request: &WithdrawalRequest,
    ) -> Result<(), Error> {
        let raw = request.raw();
        let account_script_hash: [u8; 32] = raw.account_script_hash().unpack();
        let l2_sudt_script_hash: [u8; 32] =
            build_l2_sudt_script(&ctx, &raw.sudt_script_hash().unpack()).hash();
        let amount: u128 = raw.amount().unpack();
        // find user account
        let id = self
            .get_account_id_by_script_hash(&account_script_hash.into())?
            .ok_or(AccountError::UnknownAccount)?; // find Simple UDT account
        let capacity: u64 = raw.capacity().unpack();
        // burn CKB
        self.burn_sudt(CKB_SUDT_ACCOUNT_ID, id, capacity.into())?;
        let sudt_id = self
            .get_account_id_by_script_hash(&l2_sudt_script_hash.into())?
            .ok_or(AccountError::UnknownSUDT)?;
        if sudt_id != CKB_SUDT_ACCOUNT_ID {
            // burn sudt
            self.burn_sudt(sudt_id, id, amount)?;
        } else if amount != 0 {
            return Err(WithdrawalError::WithdrawFakedCKB.into());
        }
        // increase nonce
        let nonce = self.get_nonce(id)?;
        let new_nonce = nonce.checked_add(1).ok_or(AccountError::NonceOverflow)?;
        self.set_nonce(id, new_nonce)?;
        Ok(())
    }
}
