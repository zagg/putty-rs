# Demo : un agent IA code et execute un hello world C++ dans putty-rs via MCP.
# Preconditions : putty-rs.exe lance et connecte (SSH), serveur MCP sur :3927.
# Usage : powershell -ExecutionPolicy Bypass -File scripts\mcp_hello_demo.ps1
# Voir docs/MCP_USAGE.md pour la liste complete des outils.
#
# NB : on ecrit le source avec printf (une seule ligne, \n echappes) plutot
# qu'un heredoc : un heredoc ouvert par erreur laisse bash en saisie
# multi-lignes et toutes les commandes suivantes y sont avalees (constate
# pendant la mise au point du 28/10). printf est atomique et idempotent.
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "mcp_lib.ps1")

$null = Mcp-Init
$sid = Mcp-SessionId
Write-Host "session SSH pilotee : $sid"

# 1) L'agent "frappe" le source C++ au clavier virtuel.
$write = "printf '#include <iostream>\nint main(){std::cout<<`"Hello, World! from putty-rs (pure Rust) + MCP`"<<std::endl;return 0;}\n' > hello.cpp"
Mcp-Type $sid ("cd /tmp && " + $write + [char]13)
Start-Sleep -Milliseconds 600

# 2) Compilation + execution, avec attente passive d'un motif (pas de sleep).
Mcp-Run $sid "g++ -O2 -o hello hello.cpp && ./hello && echo BUILD_OK_EXIT_0" "BUILD_OK_EXIT_0" 90000 | Out-Null

# 3) Relecture de l'ecran par l'agent : la sortie du binaire est verifiee.
$lines = Mcp-Screen $sid | Where-Object { $_.Trim() }
Write-Host "--- ecran lu via MCP ---"
$lines | Select-Object -Last 6 | ForEach-Object { "  | $_" }
if (($lines -join "`n") -match "Hello, World! from putty-rs") {
    Write-Host "[OK] hello world C++ compile (g++) et execute via putty-rs pilote en MCP" -ForegroundColor Green
} else {
    Write-Host "[KO] sortie inattendue" -ForegroundColor Red
    exit 1
}
