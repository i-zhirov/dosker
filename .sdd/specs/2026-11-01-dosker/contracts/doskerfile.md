# Doskerfile Contract

Version 1. This document defines the grammar and the semantics of every instruction. The parser and the build engine must follow it.

## Lexical Rules

- The file is UTF-8 with LF or CRLF line endings. CRLF is normalized to LF.
- An instruction is one logical line, optionally continued with a trailing backslash.
- A line whose first non-space character is `#` is a comment.
- Blank lines are ignored.
- Instruction names are case-insensitive. The conventional form is uppercase.
- The first instruction must be `FROM`.
- Arguments are separated by whitespace. Double quotes group a token and allow spaces. Inside quotes, `\"` is a literal quote and `\\` is a literal backslash.
- Leading and trailing whitespace is trimmed.
- A backslash at the end of a line continues the instruction on the next line. The backslash is removed before tokenization.

## Placeholder Expansion

- `$NAME` and `${NAME}` expand from the current `ENV` values.
- `\$` is a literal `$`.
- Expansion happens for every instruction except the name itself.
- ENV values expand from earlier ENV values.
- An undefined name is an error, unless the only occurrence is `\$`.
- Expansion happens before tokenization of instruction arguments, except that a literal `\$` is protected during tokenization.
- FROM does not expand placeholders in version 1.

## Build Context

- The context is the directory that contains the Doskerfile.
- Source paths follow the host convention. On Windows, a source may be drive-qualified, for example `C:\src`. A `COPY` destination is always a DOS path.
- `.doskerignore` uses gitignore pattern syntax.
- A `COPY` source must resolve inside the context after path normalization.
- A symlink anywhere in a `COPY` source path is an error.
- A source that does not exist is an error.
- A build context digest is part of the cache key for `COPY` and `RUN --host`.

## Grammar

```
doskerfile   = instruction+
instruction  = from | copy | run | env | workdir | cmd | machine
             | engine | engine-config | requires | expose | volume

from         = "FROM" (image_ref | import_ref | booter_ref)
image_ref    = name [":" tag | "@sha256:" hex64]
import_ref   = path                                    ; relative or absolute .img file
booter_ref   = "booter:" path

copy         = "COPY" source+ destination
source       = token
destination  = dos_path

run          = "RUN" ["--host"] command
command      = token+

env          = "ENV" key "=" value
workdir      = "WORKDIR" dos_path
cmd          = "CMD" command
machine      = "MACHINE" setting+
setting      = key "=" value
engine       = "ENGINE" engine_name
engine-config = "ENGINE-CONFIG" engine_name raw
requires     = "REQUIRES" capability ("," capability)*
expose       = "EXPOSE" port ["/" protocol]
volume       = "VOLUME" drive

engine_name  = "qemu" | "dosbox-x" | "bochs"
protocol     = "tcp"
drive        = "A:" | "B:" | "C:" | "D:" | ... | "Z:"
dos_path     = ["C:"] ("\" | "/")? component ("\" | "/" component)*
```

## Instructions

### FROM

Forms:

- `FROM freedos:<version>` builds or reuses the built-in FreeDOS base.
- `FROM <name>:<tag>` resolves a local image tag.
- `FROM <name>@sha256:<digest>` resolves a local image digest.
- `FROM ./path.img` imports a DOS image. DOSker reads the FAT tree and boot metadata. It fails if the image is not a readable DOS image.
- `FROM booter:./path.img` imports an opaque booter image. No tree, no agent.

Rules:

- FROM must be the first instruction.
- The resolved image digest is recorded in the lock file.
- A digest reference wins over a tag. A tag that moves is an error only when the lock file pins a different digest.
- An imported image is copied into the local image store as a new image. Its boot metadata is preserved.
- A booter image has `booter=true`. `COPY`, `RUN`, and in-image `exec` are rejected.
- A booter image accepts only `MACHINE`, `ENGINE`, `ENGINE-CONFIG`, and `REQUIRES`. Any other instruction is an error.
- A DOS import receives a layer that installs the agent and edits `AUTOEXEC.BAT`.

#### Agent Injection For Imported DOS Images

The import layer does this:

1. Create `C:\DOSKER` if it does not exist.
2. Copy the pinned agent to `C:\DOSKER\DOSKER.EXE`.
3. If `C:\AUTOEXEC.BAT` exists and does not already start the agent, rename it to `C:\DOSKER\AUTOEXEC.OLD`.
4. Write a new `C:\AUTOEXEC.BAT`:

```bat
@ECHO OFF
CALL C:\DOSKER\AUTOEXEC.OLD
C:\DOSKER\DOSKER.EXE
```

If the original `AUTOEXEC.BAT` does not exist, write only `@ECHO OFF` and the agent line.

#### Imported Image Requirements

- The image must have a readable FAT filesystem and a DOS kernel that can run a real-mode EXE.
- The image must use 8.3 names. Long names are not supported in version 1.
- Only one active partition is supported. A superfloppy image with no partition table is also accepted.
- The control port must not be used by a guest device driver. DOSker does not detect conflicts.
- `CONFIG.SYS` must set `LASTDRIVE` high enough for the declared volumes. DOSker does not edit `CONFIG.SYS` in version 1.
- The original `AUTOEXEC.BAT` must return. A program that never returns prevents the agent from starting.

### COPY

Forms:

- `COPY <src> <dest>`
- `COPY <src>... <dest>`

Rules:

- Sources are host paths relative to the context, or absolute paths inside the context.
- Globs are not supported in version 1.
- A source directory copies its contents into the destination, not the directory itself.
- If there are several sources, the destination is a directory and must end with `\` or `/`.
- A destination is relative to the current `WORKDIR` when it has no drive or root. `C:` is the only allowed drive.
- Every destination component is normalized to uppercase 8.3. An invalid component is an error.
- Files get the archive attribute. Directories get no special attribute. Hidden, system, and read-only attributes cannot be set in version 1.
- Timestamps are fixed to `1980-01-01T00:00:00Z`.
- Existing files are replaced. Existing directories are merged.
- The destination path must not be under `C:\DOSKER`. DOSker owns that directory for the agent, its configuration, and the wrapped `AUTOEXEC`.
- The instruction creates a layer with the added or changed files.
- The cache key includes the parent tree digest, build config digest, content hashes of the source files, and the context digest.

### RUN

Form: `RUN <command>`

Rules:

- The command runs inside DOS through the agent.
- The environment is the current image `ENV`. The working directory is the current `WORKDIR`.
- The engine is the resolved image engine, or the default QEMU.
- The build engine materializes a bootable image, boots a transient VM, waits for `HELLO`, sends `EXEC`, and streams output to the build log.
- The agent runs the command through `COMMAND.COM`, captures text output, and returns the DOS exit code. Do not use output redirection in the command. The agent captures the output.
- A non-zero exit code fails the build.
- A timeout is a failure. The default is 300 seconds and can be changed with `--timeout` on `dosker build`.
- The instruction creates a layer from the tree delta after the VM shuts down. The builder also reads the MBR, partition table, and boot sector. A change is recorded in the layer's boot metadata.
- Changes under `C:\DOSKER` are discarded. The builder restores the agent files after the step.
- A command that writes directly to video produces no captured text. The build log says so when the captured output is empty.
- The build VM uses the image machine profile. A command that uses the network is not reproducible. The docs state this.
- A command that reboots the guest fails the build with `guest_rebooted`. Reboot-based installers are not supported in version 1.
- A `RUN` on a booter image is an error.

### RUN --host

Form: `RUN --host <command>`

Rules:

- The command runs on the host in a temporary staged tree that represents `C:\`.
- The shell is `/bin/sh -c` on Unix and `cmd.exe /C` on Windows.
- The working directory is the staged tree root. A `WORKDIR` does not change the host working directory. It is provided as `DOSKER_WORKDIR`.
- The environment is a small controlled set plus `DOSKER_ROOT`, `DOSKER_WORKDIR`, and the image `ENV` values as `DOSKER_ENV_<NAME>`.
- The command has the host user's permissions and network access. It is trusted.
- A non-zero exit code fails the build.
- After exit, DOSker computes the tree delta. New and changed files become a layer. Deletions become whiteouts.
- A new or renamed path must be valid 8.3. A symlink created by the command is an error.
- Timestamps for new and changed files are fixed to `1980-01-01T00:00:00Z`.
- Changes under `C:\DOSKER` are discarded. The builder restores the agent files after the step.
- A `RUN --host` on a booter image is an error unless the command only reads. The MVP rejects it.
- The cache key includes the parent tree digest, build config digest, normalized command, and staged tree digest.

### ENV

Form: `ENV KEY=VALUE`

Rules:

- KEY matches `[A-Za-z_][A-Za-z0-9_]*`.
- VALUE is the rest of the line after `=`, with placeholder expansion applied.
- A later `ENV` with the same key replaces the value.
- ENV affects later instructions and the final image config.
- ENV is exported for `RUN`, `exec`, and `CMD`.
- ENV does not create an OCI filesystem layer. It updates the build config and adds a history entry.

### WORKDIR

Form: `WORKDIR <dos_path>`

Rules:

- The path is absolute on `C:` or relative to the current WORKDIR.
- The directory is created in the tree if it does not exist.
- WORKDIR affects later `COPY`, `RUN`, and `CMD`.
- WORKDIR does not create an OCI filesystem layer. It updates the build config and adds a history entry.

### CMD

Form: `CMD <command>`

Rules:

- CMD is a single shell-form command. It is not executed during the build.
- A later CMD replaces an earlier one.
- The supervisor sends CMD after the agent is ready when the container starts.
- CMD is not exported to `RUN`.
- CMD does not create an OCI filesystem layer.

### MACHINE

Form: `MACHINE key=value key=value ...`

Keys and defaults:

| Key | Default | Allowed |
| --- | --- | --- |
| `cpu` | `486dx2` | `8086`, `386`, `486`, `486dx2`, `pentium` |
| `memory` | `16M` | `1M..256M`, suffix `M` |
| `video` | `vga` | `cga`, `ega`, `vga`, `svga` |
| `sound` | `sb16` | `none`, `sb16` |
| `net` | `ne2000` | `none`, `ne2000` |
| `serial` | `2` | `1..4` |
| `control_port` | `2` | `1..4` |
| `disk` | `ide` | `ide`, `floppy` |
| `baud` | `115200` | `9600..921600` |

Rules:

- Repeated `MACHINE` instructions merge. A later key replaces an earlier key.
- An unknown key or an out-of-range value is an error.
- `control_port` must be less than or equal to `serial`. The agent owns that port. Every other serial port is available to the guest. The default is COM2 with two ports, so COM1 stays free.
- The effective `control_port` and `baud` are written to `C:\DOSKER\CONFIG.INI` before each boot.
- The effective profile is validated against the engine capability set before the build or run.

### ENGINE

Form: `ENGINE qemu|dosbox-x|bochs`

Rules:

- Selects the engine for build and run.
- A later ENGINE replaces an earlier one.
- `--engine` on the CLI overrides it for one command.
- The image config records the pin.

### ENGINE-CONFIG

Form: `ENGINE-CONFIG <engine> <raw text>`

Rules:

- The raw text is passed to the named adapter. QEMU splits it into command-line arguments with shell-like quoting. DOSBox-X and Bochs treat it as a configuration line.
- The instruction applies only when that engine is selected.
- It is trusted. It can request options that DOSker does not model.
- It does not change the declared capability set. A file that needs a feature must still use `REQUIRES`.
- Repeated lines are appended in order.

### REQUIRES

Form: `REQUIRES headless,gdbstub`

Rules:

- Capability names are case-sensitive and come from the adapter capability set.
- Repeated instructions merge.
- The requirement is checked after engine selection and before any VM starts.
- A missing capability is an error that names the missing capability and the engines that provide it.

### EXPOSE

Form: `EXPOSE <port>/tcp`

Rules:

- The protocol defaults to `tcp`.
- Ports are guest TCP ports.
- Repeated ports merge.
- EXPOSE is metadata. Publishing requires `-p` at run time.
- The port is recorded in the image config.

### VOLUME

Form: `VOLUME D:`

Rules:

- Declares an anonymous volume at a drive letter.
- C: is reserved for the boot disk.
- Repeated drive letters are an error.
- The volume is created at container create time if it does not exist.
- A runtime `-v name:D:` overrides the declaration.
- The declaration is recorded in the image config.

## Normalization And Cache Keys

Config-only instructions are `ENV`, `WORKDIR`, `CMD`, `MACHINE`, `ENGINE`, `ENGINE-CONFIG`, `REQUIRES`, `EXPOSE`, and `VOLUME`. They update the build config and add a history entry. They do not create a filesystem layer.

The parser produces a normalized instruction:

- Instruction name in uppercase.
- One space between tokens.
- Placeholders expanded.
- Paths normalized to uppercase 8.3 where they refer to the image tree.
- `MACHINE` keys sorted.
- `REQUIRES` capabilities sorted and deduplicated.
- Trailing whitespace removed.

The cache key is `sha256(parent_tree_digest || "\n" || build_config_digest || "\n" || normalized_instruction || "\n" || input_hashes)`. `build_config_digest` covers the effective ENV, WORKDIR, MACHINE, ENGINE, REQUIRES, EXPOSE, VOLUME, and CMD values seen so far. `input_hashes` is empty for config-only instructions, the source file hashes for `COPY`, the staged tree digest for `RUN --host`, and the parent tree digest plus engine and agent versions for `RUN`. The config digest makes an `ENV`, `WORKDIR`, or `MACHINE` change invalidate later steps.

## Errors

The parser reports the line number, the instruction, and a short reason. The build stops at the first error. A failed instruction commits no layer.
