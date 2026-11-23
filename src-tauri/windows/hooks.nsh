; NSIS hooks for the PokerLab installer (bundle > windows > nsis > installerHooks).

; An update overwrites the install dir but does not delete files that the new version no longer
; ships. Clear the bundled packs first so that solutions of older pack versions do not pile up.
; User data (installed packs, database) lives in the app data dir and is not touched.
!macro NSIS_HOOK_PREINSTALL
  StrCmp $INSTDIR "" +2
  RMDir /r "$INSTDIR\resources\packs"
!macroend
