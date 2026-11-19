param([Parameter(Mandatory)][string]$Bootstrapper)

$ErrorActionPreference = 'Stop'
$signature = Get-AuthenticodeSignature -LiteralPath $Bootstrapper
if ($signature.Status -ne 'Valid' -or
    $signature.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false) -ne 'Microsoft Corporation') {
  exit 2
}
