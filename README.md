# Kailua

Kailua uses the RISC-Zero zkVM to verifiably run Optimism's [Kona][kona] and secure rollups with cryptographic proofs for faster finality.

## Development Status

`kailua` as well as `kona` are still in active development and are NOT suitable for production usage.

## Fraud/Validity Proofs

Kailua enables rollup operators to add a new `FaultProofGame` contract, compatible with Bedrock contracts `v1.4.0` and above, using the `DisputeGameFactory` rollup instance to their deployment that relies on RISC-Zero zkVM proofs to finalize/dismiss output proposals.

`FaultProofGame` optimistically allows outputs to be accepted after a timeout if no fraud proof is published against it, or if the output is challenged, waits for a proof to be submitted to decide whether to dismiss or finalize the output.

## Prerequisites
1. [rust](https://www.rust-lang.org/tools/install)
2. [just](https://just.systems/man/en/chapter_1.html)
3. [docker](https://www.docker.com/)

## Devnet Usage

1. `just devnet-install`
   * Fetches `v1.9.1` of the `optimism` monorepo
2. `just devnet-build`
   * Builds the local cargo and foundry projects
3. `just devnet-up`
   * Starts a local OP Stack devnet using docker.
   * Dumps the output into `devnetlog.txt` for inspection.
4. `just devnet-upgrade`
   * Upgrades the devnet to use the `FaultProofGame` contract.
   * Assumes the default values of the local optimism devnet, but can take parameters.
5. Invoke Kailua to play the fault proof game
6. `just devnet-propose`
   * Launches the kailua proposer.
   * This constantly creates new `FaultProofGame` instances to advance the l2.
7. `just devnet-validate`
   * Launches the kailua validator.
   * This challenges any deployed `FaultProofGame` instance that contain invalid proposals
8. `just devnet-fault`
   * Deploys a single `FaultProofGame` instance to create a faulty proposal.
   * Tests the validator's fault proving functionality.
   * Tests the proposer's canonical chain decision functionality.
9. After you're done:
   * `just devnet-down` to stop the running docker containers
   * `just devnet-clean` to cleanup the docker volumes

## Questions, Feedback, and Collaborations

We'd love to hear from you on [Discord][discord] or [Twitter][twitter].

[bonsai access]: https://bonsai.xyz/apply
[cargo-risczero]: https://docs.rs/cargo-risczero
[crates]: https://github.com/risc0/risc0/blob/main/README.md#rust-binaries
[dev-docs]: https://dev.risczero.com
[dev-mode]: https://dev.risczero.com/api/generating-proofs/dev-mode
[discord]: https://discord.gg/risczero
[docs.rs]: https://docs.rs/releases/search?query=risc0
[examples]: https://github.com/risc0/risc0/tree/main/examples
[risc0-build]: https://docs.rs/risc0-build
[risc0-repo]: https://www.github.com/risc0/risc0
[risc0-zkvm]: https://docs.rs/risc0-zkvm
[rustup]: https://rustup.rs
[rust-toolchain]: rust-toolchain.toml
[twitter]: https://twitter.com/risczero
[zkvm-overview]: https://dev.risczero.com/zkvm
[zkhack-iii]: https://www.youtube.com/watch?v=Yg_BGqj_6lg&list=PLcPzhUaCxlCgig7ofeARMPwQ8vbuD6hC5&index=5
[kona]: https://github.com/ethereum-optimism/kona