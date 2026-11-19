# Feathrise Third-Party Patches

[简体中文](README.md) · **English**

This standalone Git repository contains patched versions of eight third-party Rust crates used by the SDK, along with native FidelityFX SDK patches and source materials. Each directory retains the upstream package name, version, original license, and existing notes about the changes.

External projects use the same patches repository URL and full commit hash for `avian3d`, `bevy_anti_alias`, `bevy_core_pipeline`, `bevy_extract`, `bevy_render`, `dlss_wgpu`, `wgpu-ffx`, and `wgpu-hal` under `[patch.crates-io]` in their root Cargo manifest. Fetch patched packages from Git and Feathrise SDK packages from crates.io.

Bevy 0.20.1 moves entity synchronization into `bevy_extract`. When a resident map is disabled and then despawned, removal observers still need access to the corresponding entities in the sub-world. The patch preserves this cleanup behavior. Chinese word segmentation now uses Bevy's official complex-script feature.

Check out the SDK and patches repositories under the same parent directory, then run `cargo check --locked -p feathrise_runtime` from the SDK directory.

The FidelityFX materials are in `fidelityfx-sdk-patches/`. The pinned native package's identity check requires the four original files to be byte-for-byte intact. For maintainer source rebuilds of the SDK and packaging from the parent repository, specify this directory explicitly with `-PatchRoot` or `FEATHRISE_FSR3_PATCH_ROOT`. Standard FSR builds read only the native ZIP you supply. The original README preserves the old commands as a record of provenance. [Current build instructions in the SDK](https://github.com/feathrise/feathrise_runtime/blob/main/crates/feathrise_fsr3_frame_generation/prebuilt/README.md)
