#!/usr/bin/env bash
set -euo pipefail

# ── colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()    { printf "${CYAN}[marlin]${NC} %s\n" "$*"; }
success() { printf "${GREEN}[marlin]${NC} %s\n" "$*"; }
warn()    { printf "${YELLOW}[marlin]${NC} %s\n" "$*"; }
die()     { printf "${RED}[marlin]${NC} %s\n" "$*" >&2; exit 1; }

# ── config ───────────────────────────────────────────────────────────────────
MARLIN_REPO="https://github.com/ByteTheBait/marlin"
AST_REPO="https://github.com/ByteTheBait/ast-compiler"
MXC_REPO="https://github.com/microsoft/mxc"

SKIP_AST=false
SKIP_MXC=false
for arg in "$@"; do
    case "$arg" in
        --no-install-ast) SKIP_AST=true ;;
        --no-install-mxc) SKIP_MXC=true ;;
        -h|--help)
            cat <<EOF
Usage: install.sh [options]

Options:
  --no-install-ast   Skip cloning/installing ast-compiler (disables AST mode)
  --no-install-mxc   Skip cloning/building mxc (disables /sandbox mxc)
  -h, --help         Show this help
EOF
            exit 0
            ;;
        *) die "Unknown option: $arg (see --help)" ;;
    esac
done

WORK_DIR="$(mktemp -d)"
trap 'info "Cleaning up source..."; rm -rf "$WORK_DIR"' EXIT

# ── install destination ───────────────────────────────────────────────────────
if [ -w /usr/local/bin ]; then
    BIN_DIR="/usr/local/bin"
elif [ -d "$HOME/.local/bin" ] || mkdir -p "$HOME/.local/bin" 2>/dev/null; then
    BIN_DIR="$HOME/.local/bin"
else
    die "Cannot find a writable bin directory. Try: sudo mkdir -p /usr/local/bin"
fi

# ── Rust ─────────────────────────────────────────────────────────────────────
INSTALLED_RUST=false
if ! command -v cargo &>/dev/null; then
    info "Rust not found — installing rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    INSTALLED_RUST=true
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
    info "Rust installed: $(rustc --version)"
else
    info "Rust already present: $(rustc --version)"
fi

# ── clone & build marlin ──────────────────────────────────────────────────────
info "Cloning marlin..."
git clone --depth 1 "$MARLIN_REPO" "$WORK_DIR/marlin"

info "Building marlin (release)..."
cargo build --release --manifest-path "$WORK_DIR/marlin/Cargo.toml"

info "Installing marlin binary → $BIN_DIR/marlin"
cp "$WORK_DIR/marlin/target/release/marlin" "$BIN_DIR/marlin"
chmod +x "$BIN_DIR/marlin"

# ── ast-compiler ─────────────────────────────────────────────────────────────
install_ast_compiler() {
    info "Cloning ast-compiler..."
    git clone --depth 1 "$AST_REPO" "$WORK_DIR/ast-compiler"

    if command -v pipx &>/dev/null; then
        info "Installing ast-compiler via pipx..."
        pipx install "$WORK_DIR/ast-compiler"
    elif command -v pip3 &>/dev/null; then
        info "Installing ast-compiler via pip3..."
        pip3 install --user "$WORK_DIR/ast-compiler"
    elif command -v pip &>/dev/null; then
        info "Installing ast-compiler via pip..."
        pip install --user "$WORK_DIR/ast-compiler"
    else
        warn "No pip/pipx found — skipping ast-compiler. Install Python 3 then re-run."
        return
    fi

    if command -v ast-compiler &>/dev/null && command -v ast-harness &>/dev/null; then
        success "ast-compiler and ast-harness installed."
    else
        warn "ast-compiler installed but commands not found in PATH."
        warn "You may need to add \$(python3 -m site --user-base)/bin to PATH."
    fi
}

if [ "$SKIP_AST" = true ]; then
    info "Skipping ast-compiler (--no-install-ast)."
else
    install_ast_compiler
fi

# ── mxc (optional sandboxing) ─────────────────────────────────────────────────
install_mxc() {
    local os arch build_script dest_name sdk_subdir
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin)
            build_script="./build-mac.sh"
            dest_name="mxc-exec-mac"
            case "$arch" in
                arm64)  sdk_subdir="arm64" ;;
                x86_64) sdk_subdir="x64"   ;;
                *)      sdk_subdir="arm64"  ;;
            esac
            ;;
        Linux)
            build_script="./build.sh"
            dest_name="lxc-exec"
            case "$arch" in
                x86_64)  sdk_subdir="x64"   ;;
                aarch64) sdk_subdir="arm64"  ;;
                *)       sdk_subdir="x64"    ;;
            esac
            ;;
        *)
            warn "mxc: unsupported OS ($os) — skipping. /sandbox mxc will not be available."
            return
            ;;
    esac

    info "Cloning mxc (microsoft/mxc)..."
    git clone --depth 1 "$MXC_REPO" "$WORK_DIR/mxc"

    info "Building mxc ($dest_name, rust-only)..."
    (
        cd "$WORK_DIR/mxc"
        chmod +x "$build_script"
        "$build_script" --rust-only
    )

    # Build scripts copy the binary into sdk/bin/<arch>/; fall back to cargo target dir
    local found_bin=""
    local sdk_path="$WORK_DIR/mxc/sdk/bin/$sdk_subdir/$dest_name"
    if [ -f "$sdk_path" ]; then
        found_bin="$sdk_path"
    else
        found_bin="$(find "$WORK_DIR/mxc/target" -name "$dest_name" -type f 2>/dev/null | head -1)"
    fi

    if [ -z "$found_bin" ]; then
        warn "mxc: build finished but binary '$dest_name' not found — skipping install."
        return
    fi

    cp "$found_bin" "$BIN_DIR/$dest_name"
    chmod +x "$BIN_DIR/$dest_name"
    success "mxc installed → $BIN_DIR/$dest_name"
}

if [ "$SKIP_MXC" = true ]; then
    info "Skipping mxc (--no-install-mxc)."
else
    install_mxc
fi

# ── clean up Rust if we installed it ─────────────────────────────────────────
if [ "$INSTALLED_RUST" = true ]; then
    info "Removing rustup (installed only for this build)..."
    rustup self uninstall -y
    success "Rust removed."
fi

# ── PATH hint ────────────────────────────────────────────────────────────────
success "marlin installed → $BIN_DIR/marlin"

if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo ""
    warn "$BIN_DIR is not in your PATH."
    warn "Add this to your shell profile (.zshrc / .bashrc):"
    warn "  export PATH=\"$BIN_DIR:\$PATH\""
fi

echo ""
success "Done! Run: marlin"
