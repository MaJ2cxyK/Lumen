; Tauri NSIS 安装器钩子：在删除程序文件之前回收沙箱资源。
; 卸载器以当前用户身份运行，无需管理员权限。
; 数据目录由 lib.rs 的 data_base() 决定：默认 %APPDATA%\Lumen，
; 旧版本迁移失败时可能仍为 %APPDATA%\AgentHub，两者都尝试清理。
!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "正在清理沙箱资源..."
  ${If} ${FileExists} "$INSTDIR\Lumen.exe"
    ${If} ${FileExists} "$APPDATA\Lumen"
      ExecWait '"$INSTDIR\Lumen.exe" --sandbox-cleanup "$APPDATA\Lumen"' $0
    ${EndIf}
    ${If} ${FileExists} "$APPDATA\AgentHub"
      ExecWait '"$INSTDIR\Lumen.exe" --sandbox-cleanup "$APPDATA\AgentHub"' $0
    ${EndIf}
  ${EndIf}
  ; 沙箱 HOME 属于容器运行数据（CLI 登录态、缓存），不是用户业务数据；
  ; 升级（/UPDATE）时保留，卸载时删除。
  ${If} $UpdateMode <> 1
    RmDir /r "$APPDATA\Lumen\sandbox-home"
    RmDir /r "$APPDATA\AgentHub\sandbox-home"
  ${EndIf}
!macroend

; Tauri 默认的“删除应用数据”只清 BUNDLEID 目录，而本应用数据目录是
; %APPDATA%\Lumen（旧版为 AgentHub）；勾选时在这里补删，静默卸载不受影响。
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    RmDir /r "$APPDATA\Lumen"
    RmDir /r "$APPDATA\AgentHub"
  ${EndIf}
!macroend
