# This script generates a summary of a Rust project within the specified root directory,
# including the project structure and the contents of all .rs and .toml files,
# excluding 'target' and hidden directories.
# The output is written to lesson-02C-distributed.md in the original directory,
# with a .txt copy created if needed.
# Set the $root variable to the desired subdirectory, e.g., "runner" or "pod\publish".
# Run this script from the root of the Rust project using PowerShell.

# Define the root directory for the search
$root = "pod\publish"

# Get the original directory
$originalDir = Get-Location

# Define the output file in the original directory
$outputFile = Join-Path $originalDir "lesson-02C-distributed-publish.md"

# Define the README path in the original directory
$readmePath = Join-Path $originalDir "README_PUBLISH.md"

# Function to generate a directory tree
function Get-DirectoryTree {
    param (
        [string]$Path = '.',
        [string]$Prefix = ''
    )
    # Get all items (files and directories) excluding 'target' and hidden items
    $items = Get-ChildItem -Path $Path | Where-Object { $_.Name -ne 'target' -and $_.Name -notlike '.*' }
    $count = $items.Count
    for ($i = 0; $i -lt $count; $i++) {
        $item = $items[$i]
        $last = $i -eq ($count - 1)
        # Use ASCII characters for tree structure
        $itemPrefix = if ($last) { '\-- ' } else { '+-- ' }
        Write-Output ($Prefix + $itemPrefix + $item.Name)
        if ($item.PSIsContainer) {
            if ($last) {
                $newPrefix = $Prefix + '    '
            } else {
                $newPrefix = $Prefix + '|   '
            }
            Get-DirectoryTree -Path $item.FullName -Prefix $newPrefix
        }
    }
}

# Check if README.md exists in the original directory and initialize the file with its content
if (Test-Path $readmePath) {
    Get-Content $readmePath | Set-Content -Path $outputFile
    Write-Output "" | Add-Content -Path $outputFile
    Write-Output "---" | Add-Content -Path $outputFile
    Write-Output "" | Add-Content -Path $outputFile
} else {
    "" | Set-Content -Path $outputFile
}

# Change to the specified root directory
Push-Location $root
try {
    # Add the Lesson 02C: Distributed section
    Write-Output "# Lesson 02C: Distributed Publish" | Add-Content -Path $outputFile
    Write-Output "" | Add-Content -Path $outputFile
    Write-Output "## Project Structure" | Add-Content -Path $outputFile
    Write-Output "" | Add-Content -Path $outputFile
    Write-Output '```' | Add-Content -Path $outputFile
    Write-Output "." | Add-Content -Path $outputFile
    Get-DirectoryTree -Path '.' -Prefix '' | Add-Content -Path $outputFile
    Write-Output '```' | Add-Content -Path $outputFile
    Write-Output "" | Add-Content -Path $outputFile

    # Find all Cargo.toml files within the root directory to identify Rust project directories
    $cargoTomls = Get-ChildItem -Path '.' -Recurse -Filter Cargo.toml -File

    # Process each Rust project
    foreach ($cargoToml in $cargoTomls) {
        $projectDir = $cargoToml.Directory.FullName
        # Find all *.toml and *.rs files, excluding those in 'target' or hidden directories
        $files = Get-ChildItem -Path $projectDir -Recurse -File -Include *.toml,*.rs | Where-Object {
            $_.FullName -notmatch '\\target\\' -and $_.FullName -notmatch '\\\.[^\\]+\\'
        }

        foreach ($file in $files) {
            # Get the relative path from the root directory
            $relativePath = Resolve-Path -Path $file.FullName -Relative

            # Read the file content as a single string
            $fileContent = Get-Content $file.FullName -Raw
            # Ensure the content ends with a newline
            if (-not $fileContent.EndsWith("`n")) {
                $fileContent += "`n"
            }
            # Create the code block
            $codeBlockOpen = switch ($file.Extension.ToLower()) {
                '.rs' { '```rust' }
                '.toml' { '```toml' }
                default { '```text' }
            }

            # Add markdown header and code block to the file
            Add-Content -Path $outputFile -Value "## $relativePath"
            Add-Content -Path $outputFile -Value ""
            Add-Content -Path $outputFile -Value $codeBlockOpen
            Add-Content -Path $outputFile -Value "`n"
            Add-Content -Path $outputFile -Value $fileContent
            Add-Content -Path $outputFile -Value '```'
            Add-Content -Path $outputFile -Value ""
        }
    }
} finally {
    # Restore the original directory
    Pop-Location
}

# If the output file does not end with .txt, create a copy with .txt appended
if (-not $outputFile.EndsWith('.txt')) {
    $txtFile = "..\lesson-02C-distributed-publish.md.txt"
    Copy-Item -Path $outputFile -Destination $txtFile
}