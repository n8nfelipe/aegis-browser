#!/usr/bin/env bash
set -Eeuo pipefail

bin_dir="${AEGIS_BIN_DIR:-${XDG_BIN_HOME:-${HOME}/.local/bin}}"
data_dir="${AEGIS_DATA_DIR:-${XDG_DATA_HOME:-${HOME}/.local/share}}"
binary_path="${bin_dir}/aegis-browser"
applications_dir="${data_dir}/applications"
icons_root="${data_dir}/icons/hicolor"
icon_dir_256="${icons_root}/256x256/apps"
icon_dir="${icons_root}/512x512/apps"
desktop_path="${applications_dir}/io.github.n8nfelipe.aegis-browser.desktop"
icon_path_256="${icon_dir_256}/io.github.n8nfelipe.aegis-browser.png"
icon_path="${icon_dir}/io.github.n8nfelipe.aegis-browser.png"

removed=0

remove_file() {
  local path="$1"
  if [[ -e "${path}" || -L "${path}" ]]; then
    rm -f -- "${path}"
    removed=1
    printf 'Removido: %s\n' "${path}"
  fi
}

remove_file "${binary_path}"
remove_file "${desktop_path}"
remove_file "${icon_path_256}"
remove_file "${icon_path}"

# Remove apenas diretórios que ficaram vazios. O index.theme e outros ícones
# pertencentes a outros aplicativos nunca são removidos.
rmdir --ignore-fail-on-non-empty "${icon_dir_256}" 2>/dev/null || true
rmdir --ignore-fail-on-non-empty "${icons_root}/256x256" 2>/dev/null || true
rmdir --ignore-fail-on-non-empty "${icon_dir}" 2>/dev/null || true
rmdir --ignore-fail-on-non-empty "${icons_root}/512x512" 2>/dev/null || true

if command -v update-desktop-database >/dev/null 2>&1 && [[ -d "${applications_dir}" ]]; then
  update-desktop-database "${applications_dir}" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1 && [[ -d "${icons_root}" ]]; then
  gtk-update-icon-cache -f -t "${icons_root}" >/dev/null 2>&1 || true
fi

if [[ "${removed}" -eq 0 ]]; then
  printf 'Aegis Browser não estava instalado nos diretórios configurados.\n'
else
  printf 'Aegis Browser desinstalado. Dados de navegação do usuário não foram removidos.\n'
fi
