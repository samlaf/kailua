// Copyright 2024 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::fault::FaultArgs;
use crate::validate::ValidateArgs;
use alloy::contract::SolCallBuilder;
use alloy::network::{Network, TransactionBuilder};
use alloy::primitives::{Address, FixedBytes, Uint, U256};
use alloy::providers::{Provider, ReqwestProvider};
use alloy::transports::Transport;
use anyhow::Context;
use deploy::DeployArgs;
use kailua_contracts::FaultProofGame::FaultProofGameInstance;
use kailua_contracts::Safe::SafeInstance;
use propose::ProposeArgs;
use std::str::FromStr;
use tracing::debug;

pub mod channel;
pub mod deploy;
pub mod fault;
pub mod propose;
pub mod validate;

pub const FAULT_PROOF_GAME_TYPE: u32 = 1337;

#[derive(clap::Parser, Debug, Clone)]
#[command(name = "kailua-cli")]
#[command(bin_name = "kailua-cli")]
#[command(author, version, about, long_about = None)]
pub enum Cli {
    Deploy(DeployArgs),
    Propose(ProposeArgs),
    Validate(ValidateArgs),
    TestFault(FaultArgs),
}

impl Cli {
    pub fn verbosity(&self) -> u8 {
        match self {
            Cli::Deploy(args) => args.v,
            Cli::Propose(args) => args.v,
            Cli::Validate(args) => args.v,
            Cli::TestFault(args) => args.propose_args.v,
        }
    }
}

pub async fn exec_safe_txn<
    T: Transport + Clone,
    P1: Provider<T, N>,
    P2: Provider<T, N>,
    C,
    N: Network,
>(
    txn: SolCallBuilder<T, P1, C, N>,
    safe: &SafeInstance<T, P2, N>,
    from: Address,
) -> anyhow::Result<()> {
    let req = txn.into_transaction_request();
    let value = req.value().unwrap_or_default();
    safe.execTransaction(
        req.to().unwrap(),
        value,
        req.input().cloned().unwrap_or_default(),
        0,
        Uint::from(req.gas_limit().unwrap_or_default()),
        U256::ZERO,
        U256::ZERO,
        Address::ZERO,
        Address::ZERO,
        [
            [0u8; 12].as_slice(),
            from.as_slice(),
            [0u8; 32].as_slice(),
            [1u8].as_slice(),
        ]
        .concat()
        .into(),
    )
    .send()
    .await?
    .get_receipt()
    .await?;
    Ok(())
}

pub async fn output_at_block(
    op_node_provider: &ReqwestProvider,
    output_block_number: u64,
) -> anyhow::Result<FixedBytes<32>> {
    let output_at_block: serde_json::Value = op_node_provider
        .client()
        .request(
            "optimism_outputAtBlock",
            (format!("0x{:x}", output_block_number),),
        )
        .await
        .context(format!("optimism_outputAtBlock {output_block_number}"))?;
    debug!("optimism_outputAtBlock {:?}", &output_at_block);
    Ok(FixedBytes::<32>::from_str(
        output_at_block["outputRoot"].as_str().unwrap(),
    )?)
}

pub async fn derive_expected_journal<T: Transport + Clone, P: Provider<T, N>, N: Network>(
    game_contract: &FaultProofGameInstance<T, P, N>,
    is_fault_proof: bool,
) -> anyhow::Result<Vec<u8>> {
    // bytes32 journalDigest = sha256(
    //     abi.encodePacked(
    //         // The L1 head hash containing the safe L2 chain data that may reproduce the L2 head hash.
    //         l1Head().raw(),
    //         // The latest finalized L2 output root.
    //         parentGame().rootClaim().raw(),
    //         // The L2 output root claim.
    //         rootClaim().raw(),
    //         // The L2 claim block number.
    //         uint64(l2BlockNumber()),
    //         // The configuration hash for this game
    //         GAME_CONFIG_HASH,
    //         // True iff the proof demonstrates fraud, false iff it demonstrates integrity
    //         isFaultProof
    //     )
    // );
    let l1_head = game_contract.l1Head().call().await?.l1Head_.0;
    let parent_contract_address = game_contract.parentGame().call().await?.parentGame_;
    let parent_contract =
        FaultProofGameInstance::new(parent_contract_address, game_contract.provider());
    let l2_output_root = parent_contract.rootClaim().call().await?.rootClaim_.0;
    let l2_claim = game_contract.rootClaim().call().await?.rootClaim_.0;
    let l2_claim_block = game_contract
        .l2BlockNumber()
        .call()
        .await?
        .l2BlockNumber_
        .to::<u64>()
        .to_be_bytes();
    let config_hash = game_contract.configHash().call().await?.configHash_.0;
    let is_fault_proof = [is_fault_proof as u8];
    Ok([
        l1_head.as_slice(),
        l2_output_root.as_slice(),
        l2_claim.as_slice(),
        l2_claim_block.as_slice(),
        config_hash.as_slice(),
        is_fault_proof.as_slice(),
    ]
    .concat())
}
