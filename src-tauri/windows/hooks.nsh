; Riptor installer hooks (Tauri NSIS). Used only by release builds, through
; src-tauri/tauri.bundle.conf.json.
;
; The VPN tab needs OpenVPN. The setup carries the official OpenVPN Community
; installer (scripts/fetch-openvpn.mjs pins its SHA-256) and installs it
; silently: the command-line client and its network drivers only, no GUI and
; no background service. An OpenVPN that is already installed is left alone,
; and uninstalling Riptor never removes OpenVPN (other programs may use it).

!include LogicLib.nsh
!include x64.nsh

; Resolved where this file is included, so File finds the MSI next to it.
!define RIPTOR_HOOKS_DIR "${__FILEDIR__}"

!macro NSIS_HOOK_POSTINSTALL
  SetRegView 64
  ReadRegStr $R0 HKLM "SOFTWARE\OpenVPN" "exe_path"
  ${If} $R0 == ""
  ${AndIfNot} ${FileExists} "$PROGRAMFILES64\OpenVPN\bin\openvpn.exe"
    DetailPrint "Installing OpenVPN (needed for Riptor's VPN)…"
    InitPluginsDir
    File "/oname=$PLUGINSDIR\openvpn.msi" "${RIPTOR_HOOKS_DIR}\openvpn\openvpn.msi"
    ExecWait '"$SYSDIR\msiexec.exe" /i "$PLUGINSDIR\openvpn.msi" /qn /norestart ADDLOCAL=OpenVPN,Drivers,Drivers.OvpnDco,Drivers.TAPWindows6' $R1
    Delete "$PLUGINSDIR\openvpn.msi"
    ${If} $R1 == 0
      DetailPrint "OpenVPN installed."
    ${ElseIf} $R1 == 3010
      DetailPrint "OpenVPN installed; Windows needs a restart to finish setting up its network driver."
    ${Else}
      DetailPrint "OpenVPN could not be installed (error $R1). Riptor works without it; only the VPN tab needs it."
    ${EndIf}
  ${Else}
    DetailPrint "OpenVPN is already installed; leaving it as it is."
  ${EndIf}
!macroend
