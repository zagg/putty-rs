#requires -Version 5.1
<#
.SYNOPSIS
    Smoke test JSON-RPC 2.0 du serveur MCP HTTP "streamable" de putty-rs.

.DESCRIPTION
    Sequence testee :
      1. initialize            (protocolVersion 2025-06-18)
      2. notifications/initialized
      3. tools/call list_sessions
      4. tools/call connection_status sur la premiere session listee

    Compatible Windows PowerShell 5.1 (pas de syntaxe pwsh 7 : pas de ternaire,
    pas de ??). Les reponses JSON et SSE (text/event-stream, lignes "data: {...}")
    sont acceptees. Le header de reponse mcp-session-id est reutilise pour les
    requetes suivantes.

.PARAMETER BaseUrl
    Endpoint MCP. Defaut : http://127.0.0.1:3927/mcp

.PARAMETER TimeoutSec
    Timeout HTTP par requete, en secondes. Defaut : 10.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\mcp_smoke.ps1

.EXAMPLE
    .\scripts\mcp_smoke.ps1 -BaseUrl http://127.0.0.1:4000/mcp -TimeoutSec 5
#>
[CmdletBinding()]
param(
    [string]$BaseUrl = 'http://127.0.0.1:3927/mcp',
    [ValidateRange(1, 600)]
    [int]$TimeoutSec = 10
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$script:TimeoutMs = $TimeoutSec * 1000
$script:NextId = 0
$script:ProtocolVersion = ''
$script:Http = $null

# ---------------------------------------------------------------------------
# Sortie compacte
# ---------------------------------------------------------------------------
function Write-Step {
    param(
        [bool]$Ok,
        [string]$Label,
        [string]$Detail = ''
    )
    if ($Ok) {
        Write-Host ('[OK]   ' + $Label) -ForegroundColor Green
    }
    else {
        Write-Host ('[FAIL] ' + $Label) -ForegroundColor Red
    }
    if (-not [string]::IsNullOrEmpty($Detail)) {
        Write-Host ('       ' + $Detail) -ForegroundColor DarkGray
    }
}

function Write-Warn {
    param(
        [string]$Label,
        [string]$Detail = ''
    )
    Write-Host ('[WARN] ' + $Label) -ForegroundColor Yellow
    if (-not [string]::IsNullOrEmpty($Detail)) {
        Write-Host ('       ' + $Detail) -ForegroundColor DarkGray
    }
}

function Format-Compact {
    param(
        $Value,
        [int]$MaxLength = 1200
    )
    if ($null -eq $Value) {
        return 'null'
    }
    if ($Value -is [System.Array] -and $Value.Count -eq 0) {
        return '[]'
    }
    $json = $Value | ConvertTo-Json -Compress -Depth 30
    if ($null -eq $json) {
        $json = [string]$Value
    }
    if ($json.Length -gt $MaxLength) {
        return $json.Substring(0, $MaxLength) + '...'
    }
    return $json
}

# ---------------------------------------------------------------------------
# HTTP / SSE
# ---------------------------------------------------------------------------
function New-McpHttpClient {
    try {
        Add-Type -AssemblyName System.Net.Http -ErrorAction Stop
    }
    catch {
        # Deja charge sur la plupart des hotes Windows PowerShell 5.1.
    }
    $handler = New-Object System.Net.Http.HttpClientHandler
    $handler.UseProxy = $false
    $handler.AllowAutoRedirect = $false
    $client = New-Object System.Net.Http.HttpClient -ArgumentList $handler
    $client.Timeout = [System.Threading.Timeout]::InfiniteTimeSpan
    return $client
}

function Get-RootException {
    param($Exception)
    $e = $Exception
    while ($null -ne $e.InnerException) {
        $e = $e.InnerException
    }
    return $e
}

function Get-HeaderValue {
    param(
        $Response,
        [string]$Name
    )
    if ($null -eq $Response) {
        return $null
    }
    if (-not $Response.Headers.Contains($Name)) {
        return $null
    }
    $values = $Response.Headers.GetValues($Name)
    foreach ($v in $values) {
        return $v
    }
    return $null
}

function Read-SseForId {
    param(
        $Response,
        [string]$ExpectedId,
        [int]$TimeoutMs
    )
    $stream = $Response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
    $reader = New-Object System.IO.StreamReader -ArgumentList $stream, ([System.Text.Encoding]::UTF8)
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    $data = New-Object System.Collections.Generic.List[string]

    try {
        while ($true) {
            $left = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
            if ($left -le 0) {
                throw ('Timeout SSE apres {0} ms' -f $TimeoutMs)
            }

            $readTask = $reader.ReadLineAsync()
            if (-not $readTask.Wait($left)) {
                throw ('Timeout SSE apres {0} ms' -f $TimeoutMs)
            }

            $line = $readTask.Result
            if ($null -eq $line) {
                break
            }

            if ($line.StartsWith('data:')) {
                $data.Add($line.Substring(5).TrimStart())
                $candidate = [string]::Join("`n", $data.ToArray())
                $obj = $null
                try {
                    $obj = $candidate | ConvertFrom-Json
                }
                catch {
                    $obj = $null
                }

                if ($null -ne $obj) {
                    $hasId = ($obj.PSObject.Properties.Name -contains 'id')
                    if ([string]::IsNullOrEmpty($ExpectedId) -or ($hasId -and ([string]$obj.id -eq $ExpectedId))) {
                        $data.Clear()
                        return $obj
                    }
                    # Evenement JSON valide mais pour un autre id (notification) :
                    # on l'ignore et on repart sur un buffer propre.
                    $data.Clear()
                }
                continue
            }

            if ($line.Length -eq 0) {
                if ($data.Count -gt 0) {
                    $candidate = [string]::Join("`n", $data.ToArray())
                    $data.Clear()
                    $obj = $null
                    try {
                        $obj = $candidate | ConvertFrom-Json
                    }
                    catch {
                        $obj = $null
                    }
                    if ($null -ne $obj) {
                        $hasId = ($obj.PSObject.Properties.Name -contains 'id')
                        if ([string]::IsNullOrEmpty($ExpectedId) -or ($hasId -and ([string]$obj.id -eq $ExpectedId))) {
                            return $obj
                        }
                    }
                }
                continue
            }
            # Lignes "event:", "id:", "retry:" et commentaires ": ..." ignorees.
        }
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }

    return $null
}

function Send-McpRequest {
    param(
        [Parameter(Mandatory = $true)]
        $Payload,
        [string]$SessionId = '',
        [string]$ExpectedId = '',
        [string]$ProtocolVersion = '',
        [int]$TimeoutMs = 0,
        [switch]$NoResponse
    )

    if ($TimeoutMs -le 0) {
        $TimeoutMs = $script:TimeoutMs
    }

    $body = $Payload | ConvertTo-Json -Compress -Depth 30
    $request = New-Object System.Net.Http.HttpRequestMessage -ArgumentList ([System.Net.Http.HttpMethod]::Post), $BaseUrl
    $content = New-Object System.Net.Http.StringContent -ArgumentList $body, ([System.Text.Encoding]::UTF8)
    $content.Headers.ContentType = [System.Net.Http.Headers.MediaTypeHeaderValue]::Parse('application/json')
    $request.Content = $content
    [void]$request.Headers.TryAddWithoutValidation('Accept', 'application/json, text/event-stream')
    if (-not [string]::IsNullOrEmpty($SessionId)) {
        [void]$request.Headers.TryAddWithoutValidation('mcp-session-id', $SessionId)
    }
    if (-not [string]::IsNullOrEmpty($ProtocolVersion)) {
        [void]$request.Headers.TryAddWithoutValidation('MCP-Protocol-Version', $ProtocolVersion)
    }

    $cts = New-Object System.Threading.CancellationTokenSource
    $cts.CancelAfter($TimeoutMs)
    $response = $null

    try {
        try {
            $response = $script:Http.SendAsync(
                $request,
                [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead,
                $cts.Token
            ).GetAwaiter().GetResult()
        }
        catch {
            $root = Get-RootException $_.Exception
            if ($root -is [System.OperationCanceledException]) {
                throw ('Timeout HTTP apres {0} ms sur {1}' -f $TimeoutMs, $BaseUrl)
            }
            throw ('Serveur MCP injoignable sur {0} : {1}. Lancer putty-rs.exe sans -nomcp, ou verifier -mcpport / McpPort.' -f $BaseUrl, $root.Message)
        }

        $status = [int]$response.StatusCode
        $mediaType = ''
        if ($null -ne $response.Content -and $null -ne $response.Content.Headers.ContentType) {
            $mediaType = [string]$response.Content.Headers.ContentType.MediaType
        }
        $responseSession = Get-HeaderValue -Response $response -Name 'mcp-session-id'

        if ($status -lt 200 -or $status -ge 300) {
            $errorBody = ''
            try {
                $errorBody = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            }
            catch {
                $errorBody = ''
            }
            if ($errorBody.Length -gt 500) {
                $errorBody = $errorBody.Substring(0, 500) + '...'
            }
            throw ('HTTP {0} sur {1} : {2}' -f $status, $BaseUrl, $errorBody)
        }

        if ($NoResponse) {
            return [pscustomobject]@{
                Status      = $status
                SessionId   = $responseSession
                Body        = $null
                Raw         = ''
                ContentType = $mediaType
            }
        }

        $parsed = $null
        $raw = ''
        if ($mediaType -like 'text/event-stream*') {
            $parsed = Read-SseForId -Response $response -ExpectedId $ExpectedId -TimeoutMs $TimeoutMs
            if ($null -eq $parsed) {
                throw ('Flux SSE termine sans reponse JSON-RPC (Content-Type: {0})' -f $mediaType)
            }
        }
        else {
            $raw = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            if (-not [string]::IsNullOrWhiteSpace($raw)) {
                try {
                    $parsed = $raw | ConvertFrom-Json
                }
                catch {
                    throw ('Reponse non JSON (Content-Type: {0}) : {1}' -f $mediaType, $raw)
                }
            }
        }

        return [pscustomobject]@{
            Status      = $status
            SessionId   = $responseSession
            Body        = $parsed
            Raw         = $raw
            ContentType = $mediaType
        }
    }
    finally {
        if ($null -ne $response) {
            $response.Dispose()
        }
        $cts.Dispose()
        $request.Dispose()
    }
}

function Invoke-McpCall {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Method,
        $Params = $null,
        [string]$SessionId = '',
        [int]$TimeoutMs = 0,
        [switch]$Notification
    )

    $payload = [ordered]@{
        jsonrpc = '2.0'
        method  = $Method
    }

    $id = $null
    if (-not $Notification) {
        $script:NextId++
        $id = $script:NextId
        $payload['id'] = $id
    }
    if ($null -ne $Params) {
        $payload['params'] = $Params
    }

    $expectedId = ''
    if (-not $Notification) {
        $expectedId = [string]$id
    }

    $response = Send-McpRequest -Payload $payload -SessionId $SessionId -ExpectedId $expectedId -ProtocolVersion $script:ProtocolVersion -TimeoutMs $TimeoutMs -NoResponse:$Notification

    if (-not $Notification) {
        if ($null -eq $response.Body) {
            throw ('Reponse JSON-RPC vide pour {0}' -f $Method)
        }
        if ($response.Body.PSObject.Properties.Name -contains 'error') {
            throw ('JSON-RPC error pour {0} : {1}' -f $Method, (Format-Compact $response.Body.error 800))
        }
    }

    return $response
}

function Invoke-Tool {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        $Arguments,
        [string]$SessionId = '',
        [int]$TimeoutMs = 0
    )
    $params = [ordered]@{
        name      = $Name
        arguments = $Arguments
    }
    return Invoke-McpCall -Method 'tools/call' -Params $params -SessionId $SessionId -TimeoutMs $TimeoutMs
}

function Get-ToolResult {
    param(
        [Parameter(Mandatory = $true)]
        $Response
    )

    $result = $null
    if ($null -ne $Response.Body -and $Response.Body.PSObject.Properties.Name -contains 'result') {
        $result = $Response.Body.result
    }

    $texts = New-Object System.Collections.Generic.List[string]
    $isError = $false

    if ($null -ne $result) {
        if ($result.PSObject.Properties.Name -contains 'isError') {
            $isError = [bool]$result.isError
        }
        if ($result.PSObject.Properties.Name -contains 'content') {
            foreach ($item in @($result.content)) {
                if ($null -ne $item -and $item.PSObject.Properties.Name -contains 'type' -and $item.type -eq 'text') {
                    $texts.Add([string]$item.text)
                }
            }
        }
    }

    $text = [string]::Join("`n", $texts.ToArray())
    $data = $null
    if (-not [string]::IsNullOrWhiteSpace($text)) {
        try {
            $data = $text | ConvertFrom-Json
        }
        catch {
            $data = $null
        }
    }
    if ($null -eq $data -and $null -ne $result -and $result.PSObject.Properties.Name -contains 'structuredContent') {
        $data = $result.structuredContent
    }

    return [pscustomobject]@{
        IsError = $isError
        Text    = $text
        Data    = $data
    }
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
$exitCode = 0
try {
    $script:Http = New-McpHttpClient
}
catch {
    Write-Step $false 'initialisation HttpClient' $_.Exception.Message
    exit 1
}

$currentStep = 'demarrage'
try {
    Write-Host ('MCP smoke -> ' + $BaseUrl) -ForegroundColor Cyan

    # 1. initialize
    $currentStep = 'initialize'
    $initParams = [ordered]@{
        protocolVersion = '2025-06-18'
        capabilities    = @{}
        clientInfo      = [ordered]@{
            name    = 'mcp-smoke'
            version = '1'
        }
    }
    $init = Invoke-McpCall -Method 'initialize' -Params $initParams
    Write-Step $true 'initialize' (Format-Compact $init.Body)

    $negotiated = ''
    if ($null -ne $init.Body -and $init.Body.PSObject.Properties.Name -contains 'result') {
        if ($null -ne $init.Body.result -and $init.Body.result.PSObject.Properties.Name -contains 'protocolVersion') {
            $negotiated = [string]$init.Body.result.protocolVersion
        }
    }
    if (-not [string]::IsNullOrEmpty($negotiated)) {
        Write-Host ('       protocolVersion negocie : ' + $negotiated) -ForegroundColor DarkGray
        $script:ProtocolVersion = $negotiated
    }
    if ($null -ne $init.Body -and $init.Body.PSObject.Properties.Name -contains 'result') {
        if ($null -ne $init.Body.result -and $init.Body.result.PSObject.Properties.Name -contains 'status' -and [string]$init.Body.result.status -eq 'starting') {
            Write-Warn 'serveur MCP en cours de demarrage (status=starting)' 'Attendre que l UI soit prete, puis relancer si les outils ne repondent pas.'
        }
    }

    $mcpSessionId = [string]$init.SessionId
    if (-not [string]::IsNullOrEmpty($mcpSessionId)) {
        Write-Step $true ('session MCP : ' + $mcpSessionId)
    }
    else {
        Write-Warn 'header mcp-session-id absent' 'Le serveur est peut-etre sans gestion de session ; poursuite sans header.'
    }

    # 2. notifications/initialized
    $currentStep = 'notifications/initialized'
    $notif = Invoke-McpCall -Method 'notifications/initialized' -SessionId $mcpSessionId -Notification
    Write-Step $true ('notifications/initialized (HTTP ' + $notif.Status + ')')

    # 3. tools/call list_sessions
    $currentStep = 'tools/call list_sessions'
    $listResponse = Invoke-Tool -Name 'list_sessions' -Arguments ([ordered]@{}) -SessionId $mcpSessionId
    $list = Get-ToolResult $listResponse
    if ($list.IsError) {
        throw ('list_sessions a retourne isError=true : ' + $list.Text)
    }
    if ($null -ne $list.Data) {
        Write-Step $true 'tools/call list_sessions' (Format-Compact $list.Data)
    }
    else {
        Write-Step $true 'tools/call list_sessions' (Format-Compact $list.Text)
    }

    $sessions = @()
    if ($null -ne $list.Data) {
        if ($list.Data.PSObject.Properties.Name -contains 'sessions') {
            $sessions = @($list.Data.sessions)
        }
        else {
            $sessions = @($list.Data)
        }
    }

    if ($sessions.Count -eq 0) {
        Write-Warn 'aucune session listee' 'connection_status non teste. Ouvrir une session (putty-rs.exe -ssh hote) puis relancer.'
    }
    else {
        $first = $sessions[0]
        $targetId = $null
        if ($null -ne $first -and $first.PSObject.Properties.Name -contains 'id') {
            $targetId = $first.id
        }
        else {
            $targetId = $first
        }

        if ($null -eq $targetId) {
            throw 'La premiere session listee ne contient pas de champ id exploitable.'
        }

        # 4. tools/call connection_status
        $currentStep = 'tools/call connection_status'
        $statusResponse = Invoke-Tool -Name 'connection_status' -Arguments ([ordered]@{ id = $targetId }) -SessionId $mcpSessionId
        $connection = Get-ToolResult $statusResponse
        if ($connection.IsError) {
            throw ('connection_status a retourne isError=true : ' + $connection.Text)
        }
        if ($null -ne $connection.Data) {
            Write-Step $true ('connection_status(' + [string]$targetId + ')') (Format-Compact $connection.Data)
        }
        else {
            Write-Step $true ('connection_status(' + [string]$targetId + ')') (Format-Compact $connection.Text)
        }
    }

    Write-Step $true 'smoke test termine'
}
catch {
    Write-Step $false ('etape en echec : ' + $currentStep) $_.Exception.Message
    $exitCode = 1
}
finally {
    if ($null -ne $script:Http) {
        $script:Http.Dispose()
    }
}

exit $exitCode
