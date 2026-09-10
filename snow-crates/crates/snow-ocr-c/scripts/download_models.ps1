$ErrorActionPreference = "Stop"

$assetDirectory = Join-Path $PSScriptRoot "..\assets\ppocrv6-small"
$artifacts = @(
    @{
        Name = "PP-OCRv6_det_small.onnx"
        Url = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.1/onnx/PP-OCRv6/det/PP-OCRv6_det_small.onnx"
        Bytes = 9929594
        Sha256 = "090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f"
    },
    @{
        Name = "PP-OCRv6_rec_small.onnx"
        Url = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.1/onnx/PP-OCRv6/rec/PP-OCRv6_rec_small.onnx"
        Bytes = 21234383
        Sha256 = "6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884"
    },
    @{
        Name = "ppocrv6_dict.txt"
        Url = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.1/paddle/PP-OCRv6/rec/PP-OCRv6_rec_small/ppocrv6_dict.txt"
        Bytes = 74947
        Sha256 = "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d"
    }
)

New-Item -ItemType Directory -Force -Path $assetDirectory | Out-Null

foreach ($artifact in $artifacts) {
    $destination = Join-Path $assetDirectory $artifact.Name
    $temporary = "$destination.download"
    try {
        Invoke-WebRequest -Uri $artifact.Url -OutFile $temporary
        $item = Get-Item -LiteralPath $temporary
        if ($item.Length -ne $artifact.Bytes) {
            throw "Unexpected size for $($artifact.Name): $($item.Length)"
        }
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $temporary).Hash.ToLowerInvariant()
        if ($hash -ne $artifact.Sha256) {
            throw "Unexpected SHA-256 for $($artifact.Name): $hash"
        }
        Move-Item -Force -LiteralPath $temporary -Destination $destination
        Write-Host "Verified $($artifact.Name) ($($item.Length) bytes)"
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -Force -LiteralPath $temporary
        }
    }
}
