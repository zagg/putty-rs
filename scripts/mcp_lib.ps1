# Biblio theque de pilotage MCP putty-rs (dot-source dans chaque shell).
$script:McpUrl = "http://127.0.0.1:3927/mcp"
$script:McpSession = $null

function Mcp-Init {
    $ir = Invoke-WebRequest -UseBasicParsing -Uri $script:McpUrl -Method Post `
        -ContentType "application/json" `
        -Headers @{ "Accept" = "application/json, text/event-stream" } `
        -Body '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"opencode-agent","version":"1"}}}' `
        -TimeoutSec 15
    $script:McpSession = $ir.Headers["mcp-session-id"]
    $null = Invoke-WebRequest -UseBasicParsing -Uri $script:McpUrl -Method Post `
        -ContentType "application/json" `
        -Headers @{ "Accept" = "application/json, text/event-stream"; "Mcp-Session-Id" = $script:McpSession } `
        -Body '{"jsonrpc":"2.0","method":"notifications/initialized"}' -TimeoutSec 10
    return $script:McpSession
}

function Mcp-Call($name, $arguments) {
    $body = @{ jsonrpc = "2.0"; id = (Get-Random); method = "tools/call"
               params = @{ name = $name; arguments = $arguments } } |
        ConvertTo-Json -Depth 6 -Compress
    $raw = (Invoke-WebRequest -UseBasicParsing -Uri $script:McpUrl -Method Post `
        -ContentType "application/json" `
        -Headers @{ "Accept" = "application/json, text/event-stream"; "Mcp-Session-Id" = $script:McpSession } `
        -Body $body -TimeoutSec 60).Content
    $data = ($raw -split "`n" | Where-Object { $_ -match "^data: \{" } | Select-Object -Last 1) -replace "^data: ", ""
    $j = $data | ConvertFrom-Json
    if ($j.result.isError) { throw "outil MCP $name en erreur : $($j.result.content[0].text)" }
    return $j.result.structuredContent
}

function Mcp-SessionId {
    return (Mcp-Call "list_sessions" @{}).sessions[0].id
}

function Mcp-Type($sid, $text) {
    $null = Mcp-Call "send_text" @{ id = $sid; text = $text }
}

# Tape une commande (CR automatique) et attend un motif regex a l'ecran.
function Mcp-Run($sid, $cmd, $regex, $timeoutMs = 45000) {
    Mcp-Type $sid ($cmd + "`r")
    $r = Mcp-Call "await_state" @{ id = $sid; regex = $regex; timeout_ms = $timeoutMs }
    if (-not $r.matched) { throw "await_state: motif '$regex' non atteint en ${timeoutMs}ms" }
    return $r
}

function Mcp-Screen($sid, $mode = "visible") {
    return (Mcp-Call "read_screen" @{ id = $sid; mode = $mode }).lines
}
