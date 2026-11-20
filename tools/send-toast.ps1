# Fire a test notification at the running RSOverlay notification server.
# Usage:
#   .\tools\send-toast.ps1 -Title "Hello" -Content "World"
#   .\tools\send-toast.ps1 -Title "Build failed" -Content "CI exited 1" -Timeout 6

param(
    [string]$Title = "RSOverlay",
    [string]$Content = "Notification test",
    [float]$Timeout = 3.0,
    [string]$SourceApp = "send-toast.ps1",
    [string]$Bind = "127.0.0.1:42069"
)

Add-Type -AssemblyName System.Net.WebSockets.Client

$payload = @{
    messageType = 1
    index = 0
    timeout = $Timeout
    height = 110.0
    opacity = 1.0
    volume = 0.7
    audioPath = "default"
    title = $Title
    content = $Content
    useBase64Icon = $false
    icon = "default"
    sourceApp = $SourceApp
} | ConvertTo-Json -Compress

$uri = [System.Uri]::new("ws://$Bind/api/v1/send")
$client = [System.Net.WebSockets.ClientWebSocket]::new()
$cts = [System.Threading.CancellationTokenSource]::new()

$client.ConnectAsync($uri, $cts.Token).Wait()
$bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
$segment = [System.ArraySegment[byte]]::new($bytes)
$client.SendAsync($segment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $cts.Token).Wait()
$client.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, "done", $cts.Token).Wait()
$client.Dispose()

Write-Host "sent: $payload"
