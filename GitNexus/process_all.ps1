$batches = 6, 7, 8
foreach ($b in $batches) {
    Write-Host "Processing batch $b..."
    python extract_batch.py $b
    node "D:\cloud\deepcloud\super agent\Understand-Anything\understand-anything-plugin\skills\understand\extract-structure.mjs" "D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/tmp/ua-file-analyzer-input-$b.json" "D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/tmp/ua-file-extract-results-$b.json"
    python generate_graph.py $b
}
Write-Host "Done."
