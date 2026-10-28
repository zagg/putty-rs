#requires -Version 5.1
<#
.SYNOPSIS
    Demo scriptee d'un login SSH pilote via le serveur MCP de putty-rs.

.DESCRIPTION
    Reproduit l'enchainement de prompts observe dans putty.exe :
      - "login as: " (auth_prompt)
      - "%s@%s's password: " ou "Password:" (le format exact depend du serveur)
      - etat "connected"

    Sequence :
      1. initialize + notifications/initialized
      2. selection d'une session (ou new_session si -HostName et aucune session)
      3. await_state { expect: "auth_prompt", regex: "^login as: " } (30 s)
      4. send_text <user> + CR
      5. await_state regex "(?i)(login as:|password:)" avec file de reponses :
         si le prompt "login as:" se repete -> renvoi du user,
         si le prompt password arrive -> send_text <password> + CR
      6. await_state { expect: "connected" } (20 s)
      7. read_screen { mode: "visible" }
      8. close_session (sauf -KeepOpen)

    Le mot de passe n'est jamais affiche ni journalise. S'il n'est pas fourni,
    il est demande via Read-Host -AsSecureString.

.PARAMETER BaseUrl
    Endpoint MCP. Defaut : http://127.0.0.1:3927/mcp

.PARAMETER TimeoutSec
    Timeout HTTP par requete courte, en secondes. Defaut : 10.

.PARAMETER SessionId
    Id de session putty-rs a piloter. Si absent, la premiere session non
    "closed" de list_sessions est utilisee.

.PARAMETER HostName
    Si aucune session n'est ouverte, cree une session SSH via new_session.

.PARAMETER Port
    Port SSH pour new_session. Defaut : 22.

.PARAMETER User
    Nom d'utilisateur SSH. Defaut : variable d'environnement PUTTY_RS_DEMO_USER.

.PARAMETER Password
    Mot de passe SSH. Defaut : variable d'environnement PUTTY_RS_DEMO_PASSWORD.
    Eviter la ligne de commande (historique) ; preferer la saisie interactive.

.PARAMETER KeepOpen
    Ne pas appeler close_session a la fin (utile pour inspecter la session).

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\mcp_demo_login.ps1 -User alice

.EXAMPLE
    .\scripts\mcp_demo_login.ps1 -User alice -SessionId 0 -BaseUrl http://127.0.0.1:3927/mcp
#>
[CmdletBinding()]
param(
    [string]$BaseUrl = 'http://127.0.0.1:3927/mcp',
    [ValidateRange(1, 600)]
    [int]$TimeoutSec = 10,
    [string]$SessionId = '',
    [string]$HostName = '',
    [ValidateRange(1, 65535)]
    [int]$Port = 22,
    [string]$User = $env:PUTTY_RS_DEMO_USER,
    [string]$Password = $env:PUTTY_RS_DEMO_PASSWORD,
    [switch]$KeepOpen
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$script:TimeoutMs = $TimeoutSec * 1000
$script:NextId = 0
$script:ProtocolVersion = ''
$script:Http = $null

$targetId = $null
$closed = $false

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

function Format-Screen {
    param(
        $Screen,
        [int]$MaxLines = 8
    )
    if ($null -eq $Screen) {
        return '(ecran vide)'
    }

    $title = ''
    $cols = 0
    $rows = 0
    $cursor = ''
    $lines = @()

    if ($Screen.PSObject.Properties.Name -contains 'title') { $title = [string]$Screen.title }
    if ($Screen.PSObject.Properties.Name -contains 'cols') { $cols = [int]$Screen.cols }
    if ($Screen.PSObject.Properties.Name -contains 'rows') { $rows = [int]$Screen.rows }
    if ($Screen.PSObject.Properties.Name -contains 'cursor') { $cursor = Format-Compact $Screen.cursor 80 }
    if ($Screen.PSObject.Properties.Name -contains 'lines') {
        $lines = @($Screen.lines | ForEach-Object {
            if ($null -ne $_ -and $_.PSObject.Properties.Name -contains 'text') {
                [string]$_.text
            }
            else {
                [string]$_
            }
        })
    }

    $nonEmpty = @($lines | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) })
    $tail = @($nonEmpty | Select-Object -Last $MaxLines)
    $tailText = @($tail | ForEach-Object {
        $s = ([string]$_).Trim()
        if ($s.Length -gt 120) { $s = $s.Substring(0, 120) + '...' }
        $s
    }) -join ' / '

    $summary = ('titre="{0}" {1}x{2} curseur={3} lignes={4}' -f $title, $cols, $rows, $cursor, $lines.Count)
    if (-not [string]::IsNullOrEmpty($tailText)) {
        $summary = $summary + ' | dernieres: ' + $tailText
    }
    return $summary
}

# ---------------------------------------------------------------------------
# HTTP / SSE (identique a mcp_smoke.ps1)
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

function Get-Matched {
    param($Data)
    if ($null -eq $Data) {
        return $false
    }
    if ($Data.PSObject.Properties.Name -contains 'matched') {
        return [bool]$Data.matched
    }
    return $false
}

function Get-Excerpt {
    param($Data)
    if ($null -eq $Data) {
        return ''
    }
    if ($Data.PSObject.Properties.Name -contains 'excerpt') {
        return [string]$Data.excerpt
    }
    return ''
}

function Get-State {
    param($Data)
    if ($null -eq $Data) {
        return ''
    }
    if ($Data.PSObject.Properties.Name -contains 'state') {
        return [string]$Data.state
    }
    return ''
}

function Get-VisibleScreenText {
    param(
        [string]$Id,
        [string]$McpSession
    )
    try {
        $response = Invoke-Tool -Name 'read_screen' -Arguments ([ordered]@{ id = $Id; mode = 'visible' }) -SessionId $McpSession
        $result = Get-ToolResult $response
        if ($result.IsError) {
            return ''
        }
        if ($null -ne $result.Data -and $result.Data.PSObject.Properties.Name -contains 'lines') {
            $lines = @($result.Data.lines | ForEach-Object { [string]$_ })
            return [string]::Join("`n", $lines)
        }
        return $result.Text
    }
    catch {
        return ''
    }
}

# ---------------------------------------------------------------------------
# Validation des identifiants (avant tout appel reseau)
# ---------------------------------------------------------------------------
if ([string]::IsNullOrEmpty($User)) {
    Write-Host '[FAIL] utilisateur manquant : passer -User ou definir PUTTY_RS_DEMO_USER.' -ForegroundColor Red
    exit 2
}

if ([string]::IsNullOrEmpty($Password)) {
    $secure = Read-Host -AsSecureString ('Mot de passe SSH pour {0}' -f $User)
    $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try {
        $Password = [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
    }
    finally {
        [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
}

if ([string]::IsNullOrEmpty($Password)) {
    Write-Host '[FAIL] mot de passe vide.' -ForegroundColor Red
    exit 2
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
    Write-Host ('MCP demo login -> ' + $BaseUrl) -ForegroundColor Cyan

    # 1. initialize
    $currentStep = 'initialize'
    $initParams = [ordered]@{
        protocolVersion = '2025-06-18'
        capabilities    = @{}
        clientInfo      = [ordered]@{
            name    = 'mcp-demo-login'
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

    # 3. selection de la session putty-rs
    $currentStep = 'selection de la session'
    if (-not [string]::IsNullOrEmpty($SessionId)) {
        $targetId = $SessionId
        Write-Step $true ('session cible fournie : ' + $targetId)
    }
    else {
        $listResponse = Invoke-Tool -Name 'list_sessions' -Arguments ([ordered]@{}) -SessionId $mcpSessionId
        $list = Get-ToolResult $listResponse
        if ($list.IsError) {
            throw ('list_sessions a retourne isError=true : ' + $list.Text)
        }
        Write-Step $true 'tools/call list_sessions' (Format-Compact $list.Data)

        $sessions = @()
        if ($null -ne $list.Data) {
            if ($list.Data.PSObject.Properties.Name -contains 'sessions') {
                $sessions = @($list.Data.sessions)
            }
            else {
                $sessions = @($list.Data)
            }
        }

        $chosen = $null
        foreach ($s in $sessions) {
            $state = ''
            if ($null -ne $s -and $s.PSObject.Properties.Name -contains 'state') {
                $state = [string]$s.state
            }
            if ($state -ne 'closed') {
                $chosen = $s
                break
            }
        }

        if ($null -ne $chosen) {
            if ($chosen.PSObject.Properties.Name -contains 'id') {
                $targetId = $chosen.id
            }
            else {
                $targetId = $chosen
            }
            Write-Step $true ('session cible : ' + [string]$targetId) (Format-Compact $chosen)
        }
        elseif (-not [string]::IsNullOrEmpty($HostName)) {
            $newResponse = Invoke-Tool -Name 'new_session' -Arguments ([ordered]@{ host = $HostName; port = $Port; proto = 'ssh' }) -SessionId $mcpSessionId
            $newSession = Get-ToolResult $newResponse
            if ($newSession.IsError) {
                throw ('new_session a echoue : ' + $newSession.Text)
            }
            Write-Step $true 'new_session' (Format-Compact $newSession.Data)

            if ($null -ne $newSession.Data -and $newSession.Data.PSObject.Properties.Name -contains 'id') {
                $targetId = $newSession.Data.id
            }
            else {
                throw ('new_session n''a pas retourne d''id exploitable : ' + $newSession.Text)
            }
        }
        else {
            throw 'Aucune session ouverte. Ouvrir putty-rs.exe -ssh <hote>, ou passer -HostName/-Port pour creer via new_session.'
        }
    }

    if ($null -eq $targetId) {
        throw 'Id de session putty-rs introuvable.'
    }

    # 4. prompt "login as: " (format observe dans putty.exe : "login as: ")
    $currentStep = 'await_state login as'
    $loginArgs = [ordered]@{
        id         = $targetId
        expect     = 'auth_prompt'
        timeout_ms = 30000
        regex      = '^login as: '
    }
    $loginResponse = Invoke-Tool -Name 'await_state' -Arguments $loginArgs -SessionId $mcpSessionId -TimeoutMs 35000
    $login = Get-ToolResult $loginResponse
    if ($login.IsError) {
        throw ('await_state(login) a echoue : ' + $login.Text)
    }
    if (-not (Get-Matched $login.Data)) {
        throw ('prompt "login as: " non detecte (timeout 30 s). Excerpt : ' + (Get-Excerpt $login.Data))
    }
    Write-Step $true 'await_state auth_prompt /^login as: /' (Format-Compact $login.Data)

    # 5. envoi du nom d'utilisateur (+ CR, comme une frappe clavier)
    $currentStep = 'send_text user'
    $userArgs = [ordered]@{
        id   = $targetId
        text = ($User + "`r")
    }
    $userResponse = Invoke-Tool -Name 'send_text' -Arguments $userArgs -SessionId $mcpSessionId
    $userResult = Get-ToolResult $userResponse
    if ($userResult.IsError) {
        throw ('send_text(user) a echoue : ' + $userResult.Text)
    }
    Write-Step $true ('send_text user "' + $User + '" + CR') (Format-Compact $userResult.Data)

    # 6. prompt password : format reel "%s@%s's password: " ou "Password: ".
    #    File de reponses : "login as:" repete -> user, "password" -> password.
    $sentPassword = $false
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        $currentStep = ('await_state password (tentative ' + $attempt + ')')
        $authArgs = [ordered]@{
            id         = $targetId
            timeout_ms = 30000
            regex      = '(?i)(login as:|password:)'
        }
        $authResponse = Invoke-Tool -Name 'await_state' -Arguments $authArgs -SessionId $mcpSessionId -TimeoutMs 35000
        $auth = Get-ToolResult $authResponse
        if ($auth.IsError) {
            throw ('await_state(auth) a echoue : ' + $auth.Text)
        }

        $authMatched = Get-Matched $auth.Data
        $excerpt = Get-Excerpt $auth.Data

        if (-not $authMatched) {
            # Repli : certains serveurs/versions peuvent ne pas exposer l'excerpt.
            $screenText = Get-VisibleScreenText -Id $targetId -McpSession $mcpSessionId
            if ($screenText -match '(?i)password') {
                $authMatched = $true
                $excerpt = 'password:'
            }
            elseif ($screenText -match '(?i)login as:') {
                $authMatched = $true
                $excerpt = 'login as:'
            }
        }

        if (-not $authMatched) {
            throw 'aucun prompt d''authentification detecte (timeout 30 s).'
        }

        Write-Step $true ('await_state prompt auth (tentative ' + $attempt + ')') (Format-Compact $auth.Data)

        if ($excerpt -match '(?i)password') {
            $currentStep = 'send_text password'
            $passwordArgs = [ordered]@{
                id   = $targetId
                text = ($Password + "`r")
            }
            $passwordResponse = Invoke-Tool -Name 'send_text' -Arguments $passwordArgs -SessionId $mcpSessionId
            $passwordResult = Get-ToolResult $passwordResponse
            if ($passwordResult.IsError) {
                throw ('send_text(password) a echoue : ' + $passwordResult.Text)
            }
            $passwordDetail = ''
            if ($null -ne $passwordResult.Data -and $passwordResult.Data.PSObject.Properties.Name -contains 'bytes_written') {
                $passwordDetail = 'bytes_written=' + [string]$passwordResult.Data.bytes_written
            }
            Write-Step $true 'send_text password (masque) + CR' $passwordDetail
            $sentPassword = $true
            break
        }

        if ($excerpt -match '(?i)login as:') {
            Write-Warn 'prompt "login as:" repete' 'renvoi du nom utilisateur'
            $userResponse = Invoke-Tool -Name 'send_text' -Arguments $userArgs -SessionId $mcpSessionId
            $userResult = Get-ToolResult $userResponse
            if ($userResult.IsError) {
                throw ('send_text(user) a echoue : ' + $userResult.Text)
            }
            Write-Step $true ('send_text user "' + $User + '" + CR (retry)') (Format-Compact $userResult.Data)
            continue
        }

        throw ('prompt non reconnu : ' + $excerpt)
    }

    if (-not $sentPassword) {
        throw 'mot de passe non envoye apres 3 tentatives.'
    }

    # 7. etat connected (20 s)
    $currentStep = 'await_state connected'
    $connectedArgs = [ordered]@{
        id         = $targetId
        expect     = 'connected'
        timeout_ms = 20000
    }
    $connectedResponse = Invoke-Tool -Name 'await_state' -Arguments $connectedArgs -SessionId $mcpSessionId -TimeoutMs 25000
    $connected = Get-ToolResult $connectedResponse
    if ($connected.IsError) {
        throw ('await_state(connected) a echoue : ' + $connected.Text)
    }
    if (-not (Get-Matched $connected.Data)) {
        throw ('etat "connected" non atteint (timeout 20 s), etat courant : ' + (Get-State $connected.Data))
    }
    Write-Step $true 'await_state connected' (Format-Compact $connected.Data)

    # 8. lecture de l'ecran visible
    $currentStep = 'read_screen'
    $screenResponse = Invoke-Tool -Name 'read_screen' -Arguments ([ordered]@{ id = $targetId; mode = 'visible' }) -SessionId $mcpSessionId
    $screen = Get-ToolResult $screenResponse
    if ($screen.IsError) {
        throw ('read_screen a echoue : ' + $screen.Text)
    }
    Write-Step $true 'read_screen visible' (Format-Screen $screen.Data)

    # 9. fermeture propre
    if ($KeepOpen) {
        Write-Warn 'close_session ignore' '-KeepOpen demande'
    }
    else {
        $currentStep = 'close_session'
        $closeResponse = Invoke-Tool -Name 'close_session' -Arguments ([ordered]@{ id = $targetId }) -SessionId $mcpSessionId
        $close = Get-ToolResult $closeResponse
        if ($close.IsError) {
            throw ('close_session a echoue : ' + $close.Text)
        }
        $closed = $true
        Write-Step $true 'close_session' (Format-Compact $close.Data)
    }

    Write-Step $true 'demo login terminee'
}
catch {
    Write-Step $false ('etape en echec : ' + $currentStep) $_.Exception.Message
    if ($null -ne $targetId -and -not $closed -and -not $KeepOpen) {
        Write-Warn ('session ' + [string]$targetId + ' laissee ouverte pour inspection') 'Fermer via close_session ou la fenetre putty-rs.'
    }
    $exitCode = 1
}
finally {
    # Ne pas laisser le mot de passe en clair dans la portee du script.
    $Password = $null
    if ($null -ne $script:Http) {
        $script:Http.Dispose()
    }
}

exit $exitCode
