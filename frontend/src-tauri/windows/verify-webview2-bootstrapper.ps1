param([Parameter(Mandatory)][string]$Bootstrapper)

# Use the Windows PowerShell security module even when NSIS was launched from PowerShell 7.
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
$ErrorActionPreference = 'Stop'
$signature = Get-AuthenticodeSignature -LiteralPath $Bootstrapper
if ($signature.Status -ne 'Valid' -or
    $signature.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false) -ne 'Microsoft Corporation') {
  exit 2
}
