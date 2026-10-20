; Installs OpenVPN (client and drivers only) unless it's already there.
; Uninstalling Riptor leaves it, since other programs may use it.

!include LogicLib.nsh
!include x64.nsh

!define RIPTOR_HOOKS_DIR "${__FILEDIR__}"
; Windows shows this file name when the uninstaller asks for admin rights.
!define RIPTOR_UNINSTALLER "Uninstall Riptor.exe"

!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\${RIPTOR_UNINSTALLER}"
  Rename "$INSTDIR\uninstall.exe" "$INSTDIR\${RIPTOR_UNINSTALLER}"
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\${RIPTOR_UNINSTALLER}$\""

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

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\${RIPTOR_UNINSTALLER}"
  RMDir "$INSTDIR"
!macroend
