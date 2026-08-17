$ErrorActionPreference = 'Stop'

$repos = Invoke-RestMethod -Uri 'https://api.github.com/users/Michaelunkai/repos?per_page=100'
$repos |
    Where-Object { $_.name -match 'startup|mich' } |
    Sort-Object updated_at -Descending |
    Select-Object name, updated_at, html_url |
    Format-Table -AutoSize
