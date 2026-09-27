#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${AEGIS_BIN_DIR:-${XDG_BIN_HOME:-${HOME}/.local/bin}}"
data_dir="${AEGIS_DATA_DIR:-${XDG_DATA_HOME:-${HOME}/.local/share}}"
binary_path="${bin_dir}/aegis-browser"
applications_dir="${data_dir}/applications"
extensions_dir="${data_dir}/aegis-browser/extensions"
icons_root="${data_dir}/icons/hicolor"
icon_dir_256="${icons_root}/256x256/apps"
icon_dir="${icons_root}/512x512/apps"
desktop_path="${applications_dir}/org.aegis.Browser.desktop"

if [[ ! -d "${repo_root}/assets" || ! -f "${repo_root}/assets/aegis-browser.png" ]]; then
  printf 'Erro: assets/aegis-browser.png não foi encontrado.\n' >&2
  exit 1
fi

if [[ "${AEGIS_SKIP_BUILD:-0}" != "1" ]]; then
  if ! command -v cargo >/dev/null 2>&1; then
    printf 'Erro: Rust/Cargo não encontrado. Defina AEGIS_SKIP_BUILD=1 para instalar um binário já compilado.\n' >&2
    exit 1
  fi

  if ! pkg-config --exists gtk+-3.0 webkit2gtk-4.1; then
    printf 'Erro: dependências GTK3/WebKitGTK 4.1 não encontradas.\n' >&2
    printf 'Instale os pacotes de desenvolvimento da sua distribuição e tente novamente.\n' >&2
    exit 1
  fi

  printf 'Compilando Aegis Browser em modo release...\n'
  cargo build --release --manifest-path "${repo_root}/Cargo.toml" -p aegis-browser-webkit
fi

source_binary="${repo_root}/target/release/aegis-browser-webkit"
if [[ ! -x "${source_binary}" ]]; then
  printf 'Erro: binário não encontrado em %s\n' "${source_binary}" >&2
  printf 'Compile com cargo build --release -p aegis-browser-webkit ou remova AEGIS_SKIP_BUILD=1.\n' >&2
  exit 1
fi

install -Dm755 "${source_binary}" "${binary_path}"
for icon_size in 48 64 128 256 512; do
  install -Dm644 "${repo_root}/assets/aegis-browser.png" \
    "${icons_root}/${icon_size}x${icon_size}/apps/org.aegis.Browser.png"
done
# Keep the theme metadata in sync on upgrades. A previous install may have
# left an index.theme that does not list all sizes shipped by this version.
install -Dm644 "${repo_root}/assets/index.theme" "${icons_root}/index.theme"
install -d "${applications_dir}" "${extensions_dir}"

# Desktop Exec entries use backslash escaping for spaces and metacharacters.
desktop_binary="$(printf '%s' "${binary_path}" | sed 's/[\\&|]/\\&/g; s/ /\\ /g')"
desktop_icon="$(printf '%s' "${icon_dir_256}/org.aegis.Browser.png" | sed 's/[\\&|]/\\&/g; s/ /\\ /g')"
sed "s|__AEGIS_BROWSER_BIN__|${desktop_binary}|g" \
  "${repo_root}/packaging/aegis-browser.desktop.in" > "${desktop_path}.tmp"
sed -i "s|__AEGIS_BROWSER_ICON__|${desktop_icon}|g" "${desktop_path}.tmp"
mv "${desktop_path}.tmp" "${desktop_path}"
chmod 644 "${desktop_path}"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${applications_dir}" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "${icons_root}" >/dev/null 2>&1 || true
fi

printf '\nAegis Browser instalado para o usuário atual.\n'
printf 'Executável: %s\n' "${binary_path}"
printf 'Atalho:     %s\n' "${desktop_path}"
printf 'Extensões:  %s\n' "${extensions_dir}"
printf 'O instalador não usa sudo e não inicia o browser automaticamente.\n'
