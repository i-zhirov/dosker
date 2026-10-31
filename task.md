# DOSker project

There is an idea for the project: a Docker-like system for launching pre-configured DOS systems.
We have to brainstorm on it.

Some already decided facts:
- it should support running on top of different execution engines -- i.e. Dosbox-X, Bochs, Qemu
- it should support pre-defining environments using Doskerfiles aking to Dockerfiles
- Doskerfile's syntax should resemble Dockerfile's one as much as it makes sense
- it should be possible to have an hierarchy of Doskerfiles -- like layers in Dockerfile
- it should be possible to have multiple containers running simultaneosly
- it should be possible to run containers in headless mode with remote control
- implementation language decided to be Rust
- ideally it should support running different DOS flavours -- like FreeDOS
- it should explose all possible debugging facilities from containers
- we have to explore possibility to expose terminal access to container using something like telnet/null-modem terminal or something

