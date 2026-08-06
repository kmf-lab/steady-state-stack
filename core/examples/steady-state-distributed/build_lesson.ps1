# build_lesson.ps1

# Define script paths using relative paths
$publishScript = ".\build_lesson-publish.ps1"
$subscribeScript = ".\build_lesson-subscribe.ps1"
$runnerScript = ".\build_lesson-runner.ps1"

# Run build_lesson-publish.ps1 if it exists
if (Test-Path $publishScript) {
    Write-Host "Running build_lesson-publish.ps1"
    & $publishScript
} else {
    Write-Host "build_lesson-publish.ps1 not found"
}

# Run build_lesson-subscribe.ps1 if it exists
if (Test-Path $subscribeScript) {
    Write-Host "Running build_lesson-subscribe.ps1"
    & $subscribeScript
} else {
    Write-Host "build_lesson-subscribe.ps1 not found"
}

# Run build_lesson-runner.sh if it exists
if (Test-Path $runnerScript) {
    Write-Host "Running build_lesson-runner.ps1"
    & $runnerScript
} else {
    Write-Host "build_lesson-runner.ps1 not found"
}