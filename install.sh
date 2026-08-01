#!/bin/sh
set -e

REPO="catsizedcoder/FigVCS"
INSTALL_DIR="$HOME/.local/bin"

echo "Downloading the latest FigVCS release..."
URL=$(curl -sSf "https://api.github.com/repos/$REPO/releases/latest" |
    grep -o '"browser_download_url": *"[^"]*fvcs-linux-x86_64.tar.gz"' |
    cut -d'"' -f4)

if [ -z "$URL" ]; then
    echo "Could not find a Linux download in the latest release."
    exit 1
fi

mkdir -p "$INSTALL_DIR"
curl -sSfL "$URL" | tar -xzf - -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR/fvcs"

echo ""
echo "FigVCS installed to $INSTALL_DIR/fvcs"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Add this line to your ~/.bashrc or ~/.zshrc so your terminal finds it:"
       echo "  export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
esac
