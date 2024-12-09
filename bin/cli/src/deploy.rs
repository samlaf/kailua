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

use crate::providers::optimism::OpNodeProvider;
use crate::stall::Stall;
use crate::KAILUA_GAME_TYPE;
use alloy::network::{EthereumWallet, TxSigner};
use alloy::primitives::{b256, Address, Bytes, Uint, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::LocalSigner;
use alloy::sol_types::SolValue;
use anyhow::Context;
use kailua_build::KAILUA_FPVM_ID;
use kailua_common::client::config_hash;
use kailua_contracts::*;
use kailua_host::fetch_rollup_config;
use risc0_zkvm::is_dev_mode;
use std::process::exit;
use std::str::FromStr;
use tracing::{error, info, warn};

#[derive(clap::Args, Debug, Clone)]
pub struct DeployArgs {
    #[arg(long, short, help = "Verbosity level (0-4)", action = clap::ArgAction::Count)]
    pub v: u8,

    /// Address of OP-NODE endpoint to use
    #[clap(long)]
    pub op_node_address: String,
    /// Address of L2 JSON-RPC endpoint to use (eth and debug namespace required).
    #[clap(long)]
    pub l2_node_address: String,
    /// Address of L1 JSON-RPC endpoint to use (eth namespace required)
    #[clap(long)]
    pub l1_node_address: String,
    /// Address of the L1 Beacon API endpoint to use.
    #[clap(long)]
    pub l1_beacon_address: Option<String>,

    /// Address of the L1 `AnchorStateRegistry` contract
    #[clap(long)]
    pub registry_contract: String,
    /// Address of the L1 `OptimismPortal` contract
    #[clap(long)]
    pub portal_contract: String,

    /// Secret key of L1 wallet to use for deploying contracts
    #[clap(long)]
    pub deployer_key: String,
    /// Secret key of L1 wallet that (indirectly) owns `DisputeGameFactory`
    #[clap(long)]
    pub owner_key: String,
    /// Secret key of L1 guardian wallet
    #[clap(long)]
    pub guardian_key: String,
}

pub async fn deploy(args: DeployArgs) -> anyhow::Result<()> {
    let op_node_provider =
        OpNodeProvider(ProviderBuilder::new().on_http(args.op_node_address.as_str().try_into()?));

    // initialize guardian wallet
    info!("Initializing guardian wallet.");
    let guardian_signer = LocalSigner::from_str(&args.guardian_key)?;
    let guardian_address = guardian_signer.address();
    let guardian_wallet = EthereumWallet::from(guardian_signer);
    let guardian_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(&guardian_wallet)
        .on_http(args.l1_node_address.as_str().try_into()?);
    let optimism_portal = OptimismPortal::new(
        Address::from_str(&args.portal_contract)?,
        &guardian_provider,
    );
    let portal_guardian_address = optimism_portal.guardian().stall().await._0;
    if portal_guardian_address != guardian_address {
        error!(
            "OptimismPortal Guardian is {portal_guardian_address}. Provided private key has account address {guardian_address}."
        );
        exit(3);
    }

    // initialize owner wallet
    info!("Initializing owner wallet.");
    let owner_signer = LocalSigner::from_str(&args.owner_key)?;
    let owner_wallet = EthereumWallet::from(owner_signer);
    let owner_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(&owner_wallet)
        .on_http(args.l1_node_address.as_str().try_into()?);

    // Init registry and factory contracts
    let anchor_state_registry =
        IAnchorStateRegistry::new(Address::from_str(&args.registry_contract)?, &owner_provider);
    info!("AnchorStateRegistry({:?})", anchor_state_registry.address());
    let dispute_game_factory = IDisputeGameFactory::new(
        anchor_state_registry.disputeGameFactory().stall().await._0,
        &owner_provider,
    );
    info!("DisputeGameFactory({:?})", dispute_game_factory.address());
    let game_count = dispute_game_factory.gameCount().stall().await.gameCount_;
    info!("There have been {game_count} games created using DisputeGameFactory");
    let dispute_game_factory_ownable = OwnableUpgradeable::new(
        anchor_state_registry.disputeGameFactory().stall().await._0,
        &owner_provider,
    );
    let factory_owner_address = dispute_game_factory_ownable.owner().stall().await._0;
    let factory_owner_safe = Safe::new(factory_owner_address, &owner_provider);
    info!("Safe({:?})", factory_owner_safe.address());
    let safe_owners = factory_owner_safe.getOwners().stall().await._0;
    info!("Safe::owners({:?})", &safe_owners);
    let owner_address = owner_wallet.default_signer().address();
    if safe_owners.first().unwrap() != &owner_address {
        error!("Incorrect owner key.");
        exit(2);
    } else if safe_owners.len() != 1 {
        error!("Expected exactly one owner of safe account.");
        exit(1);
    }

    // initialize deployment wallet
    info!("Initializing deployer wallet.");
    let deployer_signer = LocalSigner::from_str(&args.deployer_key)?;
    let deployer_wallet = EthereumWallet::from(deployer_signer);
    let deployer_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(&deployer_wallet)
        .on_http(args.l1_node_address.as_str().try_into()?);

    info!("Fetching rollup configuration from L2 nodes.");
    // fetch rollup config
    let config = fetch_rollup_config(&args.op_node_address, &args.l2_node_address, None)
        .await
        .context("fetch_rollup_config")?;
    let rollup_config_hash = config_hash(&config).expect("Configuration hash derivation error");
    info!("RollupConfigHash({})", hex::encode(rollup_config_hash));

    // Deploy verifier router contract
    info!("Deploying RiscZeroVerifierRouter contract to L1 under ownership of {owner_address}.");
    let verifier_contract = RiscZeroVerifierRouter::deploy(&deployer_provider, owner_address)
        .await
        .context("RiscZeroVerifierRouter contract deployment error")?;
    let verifier_contract =
        RiscZeroVerifierRouter::new(*verifier_contract.address(), &owner_provider);

    // Deploy RiscZeroGroth16Verifier contract
    info!("Deploying RiscZeroGroth16Verifier contract to L1.");
    // let a = ControlID::CONTROL_ROOT;
    let groth16_verifier_contract = RiscZeroGroth16Verifier::deploy(
        &deployer_provider,
        b256!("8cdad9242664be3112aba377c5425a4df735eb1c6966472b561d2855932c0469"),
        b256!("04446e66d300eb7fb45c9726bb53c793dda407a62e9601618bb43c5c14657ac0"),
    )
    .await
    .context("RiscZeroGroth16Verifier contract deployment error")?;
    info!("{:?}", &groth16_verifier_contract);
    let selector = groth16_verifier_contract.SELECTOR().stall().await._0;
    info!("Adding RiscZeroGroth16Verifier contract to RiscZeroVerifierRouter.");
    verifier_contract
        .addVerifier(selector, *groth16_verifier_contract.address())
        .send()
        .await
        .context("addVerifier RiscZeroGroth16Verifier (send)")?
        .get_receipt()
        .await
        .context("addVerifier RiscZeroGroth16Verifier (get_receipt)")?;

    // Deploy mock verifier
    if is_dev_mode() {
        // Deploy MockVerifier contract
        warn!("Deploying RiscZeroMockVerifier contract to L1. This will accept fake proofs which are not cryptographically secure!");
        let mock_verifier_contract =
            RiscZeroMockVerifier::deploy(&deployer_provider, [0u8; 4].into())
                .await
                .context("RiscZeroMockVerifier contract deployment error")?;
        warn!("{:?}", &mock_verifier_contract);
        warn!("Adding RiscZeroMockVerifier contract to RiscZeroVerifierRouter.");
        verifier_contract
            .addVerifier([0u8; 4].into(), *mock_verifier_contract.address())
            .send()
            .await
            .context("addVerifier RiscZeroMockVerifier (send)")?
            .get_receipt()
            .await
            .context("addVerifier RiscZeroMockVerifier (get_receipt)")?;
    }

    // Deploy KailuaTreasury contract
    info!("Deploying KailuaTreasury contract to L1 rpc.");
    let fault_dispute_game_type = 254;
    let kailua_treasury_implementation = KailuaTreasury::deploy(
        &deployer_provider,
        *verifier_contract.address(),
        bytemuck::cast::<[u32; 8], [u8; 32]>(KAILUA_FPVM_ID).into(),
        rollup_config_hash.into(),
        Uint::from(64),
        KAILUA_GAME_TYPE,
        Address::from_str(&args.registry_contract)?,
    )
    .await
    .context("KailuaTreasury implementation contract deployment error")?;
    info!("{:?}", &kailua_treasury_implementation);

    // Update dispute factory implementation to KailuaTreasury
    info!("Setting KailuaTreasury initialization bond value in DisputeGameFactory to zero.");
    crate::exec_safe_txn(
        dispute_game_factory.setInitBond(KAILUA_GAME_TYPE, U256::ZERO),
        &factory_owner_safe,
        owner_address,
    )
    .await
    .context("setInitBond 0 wei")?;
    assert_eq!(
        dispute_game_factory
            .initBonds(KAILUA_GAME_TYPE)
            .stall()
            .await
            .bond_,
        U256::ZERO
    );
    info!("Setting KailuaTreasury particpation bond value to 1 wei.");
    let bond_value = U256::from(1);
    crate::exec_safe_txn(
        kailua_treasury_implementation.setParticipationBond(bond_value),
        &factory_owner_safe,
        owner_address,
    )
    .await
    .context("setParticipationBond 1 wei")?;
    assert_eq!(
        kailua_treasury_implementation
            .participationBond()
            .stall()
            .await
            ._0,
        bond_value
    );

    info!("Setting KailuaTreasury implementation address in DisputeGameFactory.");
    crate::exec_safe_txn(
        dispute_game_factory
            .setImplementation(KAILUA_GAME_TYPE, *kailua_treasury_implementation.address()),
        &factory_owner_safe,
        owner_address,
    )
    .await
    .context("setImplementation KailuaTreasury")?;
    assert_eq!(
        dispute_game_factory
            .gameImpls(KAILUA_GAME_TYPE)
            .stall()
            .await
            .impl_,
        *kailua_treasury_implementation.address()
    );

    // Create new treasury
    let fault_dispute_anchor = anchor_state_registry
        .anchors(fault_dispute_game_type)
        .stall()
        .await;
    let root_claim_number: u64 = fault_dispute_anchor._1.to();
    let root_claim = op_node_provider.output_at_block(root_claim_number).await?;

    let extra_data = Bytes::from(root_claim_number.abi_encode_packed());
    // Skip setup if target anchor already exists
    let existing_treasury_address = dispute_game_factory
        .games(KAILUA_GAME_TYPE, root_claim, extra_data.clone())
        .stall()
        .await
        .proxy_;
    if existing_treasury_address.is_zero() {
        info!(
            "Creating new KailuaTreasury game instance from {} ({}).",
            fault_dispute_anchor._1, root_claim
        );
        dispute_game_factory
            .create(KAILUA_GAME_TYPE, root_claim, extra_data.clone())
            .send()
            .await
            .context("create KailuaTreasury (send)")?
            .get_receipt()
            .await
            .context("create KailuaTreasury (get_receipt)")?;
    } else {
        info!(
            "Already found a game instance for anchor {}.",
            fault_dispute_anchor._1
        );
    }
    let kailua_treasury_instance_address = dispute_game_factory
        .games(KAILUA_GAME_TYPE, root_claim, extra_data)
        .stall()
        .await
        .proxy_;
    let kailua_treasury_instance =
        KailuaTreasury::new(kailua_treasury_instance_address, &owner_provider);
    info!("{:?}", &kailua_treasury_instance);
    let status = kailua_treasury_instance.status().stall().await._0;
    if status == 0 {
        info!("Resolving KailuaTreasury instance");
        kailua_treasury_instance
            .resolve()
            .send()
            .await
            .context("KailuaTreasury::resolve (send)")?
            .get_receipt()
            .await
            .context("KailuaTreasury::resolve (get_receipt)")?;
    } else {
        info!("Game instance is not ongoing ({status})");
    }

    // Deploy KailuaGame contract
    info!("Deploying KailuaGame contract to L1 rpc.");
    let kailua_game_contract = KailuaGame::deploy(
        &deployer_provider,
        *kailua_treasury_implementation.address(),
        *verifier_contract.address(),
        bytemuck::cast::<[u32; 8], [u8; 32]>(KAILUA_FPVM_ID).into(),
        rollup_config_hash.into(),
        Uint::from(64),
        KAILUA_GAME_TYPE,
        Address::from_str(&args.registry_contract)?,
        U256::from(config.genesis.l2_time),
        U256::from(config.block_time),
        U256::from(24),
        300,
    )
    .await
    .context("KailuaGame contract deployment error")?;
    info!("{:?}", &kailua_game_contract);

    // Update implementation to KailuaGame
    info!("Setting KailuaGame implementation address in DisputeGameFactory.");
    crate::exec_safe_txn(
        dispute_game_factory.setImplementation(KAILUA_GAME_TYPE, *kailua_game_contract.address()),
        &factory_owner_safe,
        owner_address,
    )
    .await
    .context("setImplementation KailuaGame")?;
    // Update the respectedGameType as the guardian
    info!("Setting respectedGameType in OptimismPortal.");
    optimism_portal
        .setRespectedGameType(KAILUA_GAME_TYPE)
        .send()
        .await
        .context("setImplementation KailuaGame")?
        .get_receipt()
        .await?;
    info!("Kailua upgrade complete.");
    Ok(())
}
