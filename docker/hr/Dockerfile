# syntax=docker/dockerfile:1
# htui-hr-dev — toolchain for one /handoff-run sandbox (TOOL-7). Built only by `scripts/hr build`:
#   docker build -t htui-hr-dev -f docker/hr/Dockerfile \
#     --build-arg HR_USER=$(id -un) --build-arg HR_UID=$(id -u) --build-arg HR_GID=$(id -g) \
#     --build-arg HR_HOME=$HOME docker/hr
# Host tools (claude, gortex, graphify, headroom, uv/uvx, serena) are NOT baked in: their ~/.local
# dirs (bin, share/mise/installs, share/uv/{python,tools}) are mounted read-only so a sandbox never
# drifts from the host; the rest of ~/.local is never mounted (it holds secrets). Design: .claude/plans/tool-7-hr-sandbox.plan.md
FROM ubuntu:24.04 AS base
ARG HR_USER=mluigi
ARG HR_UID=1000
ARG HR_GID=1000
ARG HR_HOME=/home/mluigi
ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8
# libdbus-1-dev: keyring -> dbus-secret-service; libssl-dev: native-tls (hf-hub/fastembed, reqwest).
# postgresql-client-16 is in noble/main; no PGDG repo needed. PGDG alternative, only if a newer
# 16.x minor is ever wanted:
#   install -d /usr/share/postgresql-common/pgdg && curl -fsSLo /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc https://www.postgresql.org/media/keys/ACCC4CF8.asc
#   echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] https://apt.postgresql.org/pub/repos/apt noble-pgdg main" >/etc/apt/sources.list.d/pgdg.list
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git jq less procps ripgrep \
      build-essential pkg-config libdbus-1-dev libssl-dev \
      postgresql-client-16 \
 && rm -rf /var/lib/apt/lists/*
# ubuntu:24.04 ships user `ubuntu` at 1000:1000, which collides with the host uid. Bind points under
# $HOME are pre-created user-owned, otherwise Docker creates them as root.
# The skel ~/.profile prepends ~/.local/bin to PATH in login shells, which would put the host's stale
# uv/uvx ahead of the mise dirs the compose file puts first; replace it with one that leaves PATH alone.
RUN (userdel -r ubuntu 2>/dev/null || true) \
 && (getent group "$HR_GID" >/dev/null || groupadd -g "$HR_GID" "$HR_USER") \
 && useradd -m -d "$HR_HOME" -u "$HR_UID" -g "$HR_GID" -s /bin/bash "$HR_USER" \
 && printf '%s\n' '# htui-hr-dev: PATH is set by compose.hr.yaml; do not prepend ~/.local/bin here.' \
      '[ -n "$BASH_VERSION" ] && [ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"' >"$HR_HOME/.profile" \
 && chown "$HR_UID:$HR_GID" "$HR_HOME/.profile" \
 && install -d -o "$HR_UID" -g "$HR_GID" /opt/rust \
      "$HR_HOME/.cache" "$HR_HOME/.config" "$HR_HOME/.gortex" "$HR_HOME/.cargo" "$HR_HOME/projects" \
      "$HR_HOME/.local" "$HR_HOME/.local/share" "$HR_HOME/.local/share/mise" "$HR_HOME/.local/share/uv" \
      "$HR_HOME/.local/bin" "$HR_HOME/.local/share/mise/installs" "$HR_HOME/.local/share/uv/python" \
      "$HR_HOME/.local/share/uv/tools" "$HR_HOME/.local/state" \
 && install -d /host /hr-state /hr-seed

FROM base AS dev
ARG HR_USER
ARG HR_UID
ARG HR_GID
ARG HR_HOME
USER ${HR_UID}:${HR_GID}
# Build-time cargo lives in /opt/rust; at run time CARGO_HOME is the shared htui-hr-cargo volume.
ENV RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo PATH=/opt/rust/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
# Pin = rust-toolchain.toml; its components omit rust-analyzer (serena's Rust LSP), added here.
RUN curl --proto '=https' --tlsv1.2 -sSfo /tmp/rustup-init.sh https://sh.rustup.rs \
 && sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain 1.98.1 \
 && rm /tmp/rustup-init.sh \
 && rustup component add rustfmt clippy rust-analyzer --toolchain 1.98.1
# Versions match Cargo.lock: sqlx 0.9.0, insta 1.48.0.
RUN cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,rustls \
 && cargo install cargo-insta --version 1.48.0 --locked \
 && rm -rf "$CARGO_HOME/registry" "$CARGO_HOME/git"
# Runtime CARGO_HOME = shared htui-hr-cargo volume (whole, incl. .package-cache locks).
ENV CARGO_HOME=${HR_HOME}/.cargo HOME=${HR_HOME} USER=${HR_USER} SHELL=/bin/bash
LABEL hr.uid=${HR_UID} hr.gid=${HR_GID} hr.home=${HR_HOME} hr.toolchain=1.98.1 hr.sqlx-cli=0.9.0 hr.cargo-insta=1.48.0
WORKDIR ${HR_HOME}
CMD ["sleep", "infinity"]
