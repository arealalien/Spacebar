!macro NSIS_HOOK_POSTINSTALL
  ; The Tauri updater sets $UpdateMode before launching this section.
  Push $0
  Push $1
  StrCpy $0 0
  IfErrors 0 +2
  StrCpy $0 1

  ${If} $UpdateMode = 1
    CreateDirectory "$APPDATA\${BUNDLEID}"
    ClearErrors
    FileOpen $1 "$APPDATA\${BUNDLEID}\whats-new-version" w
    ${IfNot} ${Errors}
      FileWrite $1 "${VERSION}"
      FileClose $1
    ${EndIf}
  ${Else}
    Delete "$APPDATA\${BUNDLEID}\whats-new-version"
  ${EndIf}

  ${If} $0 = 1
    SetErrors
  ${Else}
    ClearErrors
  ${EndIf}
  Pop $1
  Pop $0
!macroend
