# Runtime Tree-Layer Projection Implementation Plan

- **Date**: 2026-11-23
- **Status**: Implemented
- **Design**: [DOSker design](design.md)
- **Depends on**: —
- **Manual prerequisites**: A prepared QEMU image for the `qemu_build` integration test.
- **Goal**: Make `create` and `run` materialize the final image tree onto the private C: disk, so `COPY` and in-DOS `RUN` changes are visible at runtime.

## Design

`create` already copies the image raw-disk blob. The existing
`dosker_image::project_tree(disk, tree)` writes the final canonical tree
onto that copied disk. Use it after the raw-disk copy for non-booter
images whose history contains a non-empty `COPY` or `RUN` entry.

This keeps config-only, import-only, and base images byte-identical and
fast. It skips booter images. It does not apply layer boot overrides.
The projected disk is runtime state only; the image and its raw-disk
annotation stay unchanged.

## Constraints

- Project only when `!config.booter`.
- Project only when history has a non-empty entry starting with `COPY ` or `RUN `.
- Use `dosker_image::project_tree`; do not add another projection implementation.
- Project after `copy_disk_to` and before the record is saved with its disk path.
- On projection failure, remove anonymous volumes and the container record.
- Booter images remain byte-identical copies.
- Import-only and config-only images remain byte-identical copies.
- Do not apply boot overrides in this change.
- No manifest, config schema, archive, or save/load changes.
- The image's stored raw disk is never modified.

## Tasks

- [x] **Task 1: Add the projection predicate**
    - **Implements:** Design §Runtime tree projection; Constraints
    - **Modify `crates/dosker-cli/src/commands/create.rs`**
        - Import `HistoryEntry`, `ImageConfig`, and `project_tree`.
        - Add `fn has_tree_changes(history: &[HistoryEntry]) -> bool`:
          true when any entry has `!empty_layer` and `created_by` starts
          with `COPY ` or `RUN `.
        - Add `fn needs_tree_projection(config: &ImageConfig) -> bool`:
          `!config.booter && has_tree_changes(&config.history)`.
        - Add unit tests in the existing `mod tests`:
          `copy_and_run_history_need_projection`,
          `config_only_history_does_not_need_projection`, and
          `empty_copy_and_run_layers_do_not_need_projection`.
    - **Verify:** `cargo test -p dosker-cli --lib copy_and_run_history_need_projection`

- [x] **Task 2: Project the tree in `create`**
    - **Implements:** Design §Runtime tree projection; Constraints
    - **Modify `crates/dosker-cli/src/commands/create.rs`**
        - Load the image config after the image.
        - Add `fn project_create_disk(context, reference, disk) -> Result<(), CliError>`
          that calls `context.images.load_tree(reference)` and
          `dosker_image::project_tree(disk, &tree)`.
        - After `copy_disk_to`, call `project_create_disk` when
          `needs_tree_projection(&config)` is true.
        - On error, call `remove_anonymous` and `context.store.remove(&record)`
          before returning, matching the raw-copy failure path.
    - **Verify:** `cargo check -p dosker-cli --all-targets`

- [x] **Task 3: Cover projection without QEMU**
    - **Implements:** Design §Runtime tree projection; Constraints
    - **Modify `crates/dosker-cli/tests/cli_build.rs`**
        - Add `create_projects_copy_layers_onto_the_private_disk`:
          import `base:latest`, build `COPY ADD.TXT C:/APPS/`, create,
          read the private disk with `dosker_image::read_tree`, and
          assert `C:\APPS\ADD.TXT` bytes.
        - Add `create_rejects_a_tree_that_does_not_fit_the_disk`:
          build a `COPY` tree larger than the 32 MiB base, create, assert
          exit 1, and assert `dosker ps -a` has no container.
    - **Modify `crates/dosker-cli/tests/cli_lifecycle.rs`**
        - Extend `create_records_the_digest_and_booter_flag` to assert
          the booter private disk is byte-identical to the booter blob.
    - **Verify:**
      - `cargo test -p dosker-cli --features mock-engine --test cli_build create_projects_copy_layers_onto_the_private_disk`
      - `cargo test -p dosker-cli --features mock-engine --test cli_build create_rejects_a_tree_that_does_not_fit_the_disk`
      - `cargo test -p dosker-cli --features mock-engine --test cli_lifecycle create_records_the_digest_and_booter_flag`

- [x] **Task 4: Prove COPY and RUN changes reach the guest**
    - **Implements:** Design §Runtime tree projection
    - **Modify `crates/dosker-cli/tests/qemu_build.rs`**
        - In `built_image_boots_and_answers_exec`, after the container
          starts, run `TYPE C:\APPS\HELLO.TXT` and
          `TYPE C:\GEN.TXT`.
        - Assert `COPIED` and `HOST` appear.
        - Replace the stale comment that says the runtime boots the
          retained base disk without tree changes.
    - **Verify:**
      - `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img cargo test -p dosker-cli --test qemu_build built_image_boots_and_answers_exec`

- [x] **Task 5: Document runtime projection**
    - **Implements:** Design §Runtime tree projection; Constraints
    - **Modify `docs/builds.md`**
        - Add a runtime projection section: non-booter images with
          non-empty `COPY`/`RUN` history project the final tree at
          create/run; config-only, import-only, and booter images keep
          the raw-disk copy.
        - State that boot overrides are not applied and the stored image
          is not modified.
    - **Modify `docs/images.md`**
        - Explain that non-booter container disks can be a projected copy
          of the raw base disk plus the final tree.
        - Update the `copy_disk_to` materialization note.
    - **Modify `examples/openwatcom-dos-bundled/PLAN.md`**
        - Mark the old imported-disk workaround as predating runtime tree
          projection; the example remains the prebuilt-disk variant.
    - **Verify:**
      - `rg -n "runtime.*project|tree.*project|projection" docs/builds.md docs/images.md`
      - `git diff --check`

- [x] **Task 6: Run the focused and workspace checks**
    - **Implements:** All constraints
    - **Verify:**
      - `cargo fmt --all --check`
      - `cargo test -p dosker-cli --lib`
      - `cargo test -p dosker-image -p dosker-build`
      - `cargo test -p dosker-cli --features mock-engine --test cli_build --test cli_lifecycle`
      - `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img cargo test -p dosker-cli --test qemu_build built_image_boots_and_answers_exec`
    - **Final review:** self-review the create/runtime diff, confirm no
      stored image or boot metadata changed, and confirm the qemu test
      observes the projected files in the guest.

## Result

- `cargo fmt --all --check` passed.
- `cargo test -p dosker-cli --lib` passed: 43 tests.
- `cargo test -p dosker-cli --features mock-engine --test cli_build --test cli_lifecycle` passed: 17 build tests and 24 lifecycle tests.
- `cargo test -p dosker-image -p dosker-build` passed.
- `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img cargo test -p dosker-cli --test qemu_build built_image_boots_and_answers_exec` passed. The guest read `COPIED` from `C:\APPS\HELLO.TXT` and `HOST` from `C:\GEN.TXT`.
- The booter copy, import-only copy, config-only skip, and oversized-tree cleanup tests passed.
- The stored image raw disk and boot metadata are unchanged by `create`; only the private container disk is projected.
- Docs and the historical bundled-example plan were updated.

## Final Review

The QEMU integration test observed the projected `COPY` and `RUN --host`
files in the guest. The non-QEMU tests prove the booter/import-only
skip and failure cleanup. The plan is implemented.
