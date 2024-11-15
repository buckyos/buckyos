function Convert-Subnetmask {
    [CmdLetBinding(DefaultParameterSetName = 'CIDR')]
    param( 
        [Parameter( 
            ParameterSetName = 'CIDR',       
            Position = 0,
            Mandatory = $true,
            HelpMessage = 'CIDR like /24 without "/"')]
        [ValidateRange(0, 32)]
        [Int32]$CIDR,

        [Parameter(
            ParameterSetName = 'Mask',
            Position = 0,
            Mandatory = $true,
            HelpMessage = 'Subnetmask like 255.255.255.0')]
        [ValidateScript({
                if ($_ -match "^(254|252|248|240|224|192|128).0.0.0$|^255.(254|252|248|240|224|192|128|0).0.0$|^255.255.(254|252|248|240|224|192|128|0).0$|^255.255.255.(255|254|252|248|240|224|192|128|0)$") {
                    return $true
                }
                else {
                    throw "Enter a valid subnetmask (like 255.255.255.0)!"    
                }
            })]
        [String]$Mask
    )

    Begin {

    }

    Process {
        switch ($PSCmdlet.ParameterSetName) {
            "CIDR" {                          
                # Make a string of bits (24 to 11111111111111111111111100000000)
                $CIDR_Bits = ('1' * $CIDR).PadRight(32, "0")
                
                # Split into groups of 8 bits, convert to Ints, join up into a string
                $Octets = $CIDR_Bits -split '(.{8})' -ne ''
                $Mask = ($Octets | ForEach-Object -Process { [Convert]::ToInt32($_, 2) }) -join '.'
            }

            "Mask" {
                # Convert the numbers into 8 bit blocks, join them all together, count the 1
                $Octets = $Mask.ToString().Split(".") | ForEach-Object -Process { [Convert]::ToString($_, 2) }
                $CIDR_Bits = ($Octets -join "").TrimEnd("0")

                # Count the "1" (111111111111111111111111 --> /24)                     
                $CIDR = $CIDR_Bits.Length             
            }               
        }

        [pscustomobject] @{
            Mask = $Mask
            CIDR = $CIDR
        }
    }

    End {
        
    }
}

# Helper function to convert an IPv4-Address to Int64 and vise versa
function Convert-IPv4Address {
    [CmdletBinding(DefaultParameterSetName = 'IPv4Address')]
    param(
        [Parameter(
            ParameterSetName = 'IPv4Address',
            Position = 0,
            Mandatory = $true,
            HelpMessage = 'IPv4-Address as string like "192.168.1.1"')]
        [IPaddress]$IPv4Address,

        [Parameter(
            ParameterSetName = 'Int64',
            Position = 0,
            Mandatory = $true,
            HelpMessage = 'IPv4-Address as Int64 like 2886755428')]
        [long]$Int64
    ) 

    Begin {

    }

    Process {
        switch ($PSCmdlet.ParameterSetName) {
            # Convert IPv4-Address as string into Int64
            "IPv4Address" {
                $Octets = $IPv4Address.ToString().Split(".") 
                $Int64 = [long]([long]$Octets[0] * 16777216 + [long]$Octets[1] * 65536 + [long]$Octets[2] * 256 + [long]$Octets[3]) 
            }
    
            # Convert IPv4-Address as Int64 into string 
            "Int64" {            
                $IPv4Address = (([System.Math]::Truncate($Int64 / 16777216)).ToString() + "." + ([System.Math]::Truncate(($Int64 % 16777216) / 65536)).ToString() + "." + ([System.Math]::Truncate(($Int64 % 65536) / 256)).ToString() + "." + ([System.Math]::Truncate($Int64 % 256)).ToString())
            }      
        }

        [pscustomobject] @{   
            IPv4Address = $IPv4Address
            Int64       = $Int64
        }
    }

    End {

    }
}

# Helper function to create a new Subnet
function Get-IPv4Subnet {
    param(
        [Parameter(
            Position = 0,
            Mandatory = $true,
            HelpMessage = 'IPv4-Address which is in the subnet')]
        [IPAddress]$IPv4Address,

        [Parameter(
            ParameterSetName = 'CIDR',
            Position = 1,
            Mandatory = $true,
            HelpMessage = 'CIDR like /24 without "/"')]
        [ValidateRange(0, 31)]
        [Int32]$CIDR
    )

    Begin {
    
    }

    Process {
        # Get CIDR Address by parsing it into an IP-Address
        $CIDRAddress = [System.Net.IPAddress]::Parse([System.Convert]::ToUInt64(("1" * $CIDR).PadRight(32, "0"), 2))
    
        # Binary AND ... this is how subnets work.
        $NetworkID_bAND = $IPv4Address.Address -band $CIDRAddress.Address

        # Return an array of bytes. Then join them.
        $NetworkID = [System.Net.IPAddress]::Parse([System.BitConverter]::GetBytes([UInt32]$NetworkID_bAND) -join ("."))
        
        # Get HostBits based on SubnetBits (CIDR) // Hostbits (32 - /24 = 8 -> 00000000000000000000000011111111)
        $HostBits = ('1' * (32 - $CIDR)).PadLeft(32, "0")
        
        # Convert Bits to Int64
        $AvailableIPs = [Convert]::ToInt64($HostBits, 2)

        # Convert Network Address to Int64
        $NetworkID_Int64 = (Convert-IPv4Address -IPv4Address $NetworkID.ToString()).Int64

        # Convert add available IPs and parse into IPAddress
        $Broadcast = [System.Net.IPAddress]::Parse((Convert-IPv4Address -Int64 ($NetworkID_Int64 + $AvailableIPs)).IPv4Address)
        
        # Change useroutput ==> (/27 = 0..31 IPs -> AvailableIPs 32)
        $AvailableIPs += 1

        # Hosts = AvailableIPs - Network Address + Broadcast Address
        $Hosts = ($AvailableIPs - 2)
            
        # Build custom PSObject
        [pscustomobject] @{
            NetworkID = $NetworkID
            Broadcast = $Broadcast
            IPs       = $AvailableIPs
            Hosts     = $Hosts
        }
    }

    End {

    }
}

function Get-LocalIPAddresses {
    $networkAdapters = Get-NetIPAddress | Where-Object { $_.AddressState -eq 'Preferred' -and $_.AddressFamily -eq 'IPv4' -and $_.IPAddress -ne '127.0.0.1' }
    $networkAdapters | ForEach-Object {
        [PSCustomObject]@{
            IPAddress  = $_.IPAddress
            SubnetMask = $_.PrefixLength
        }
    }
}

Write-Output "Scanning local network for devices"
$localIps = Get-LocalIPAddresses
Write-Output "Found $($localIps.Count) local IP addresses"

$Threads = 128

function Test-Port {
    param (
        [string]$IPAddress
    )
    Write-Output "Testing port 3180 on $IPAddress"

    $port = 3180
    $timeout = 1000

    try {
        $tcpClient = New-Object System.Net.Sockets.TcpClient
        if ($tcpClient.ConnectAsync($IPAddress, $port).Wait($timeout)) {
            $tcpClient.Close()
            Write-Output $IPAddress
        }
    }
    catch {
        # do nothing
    }
}

# Scriptblock --> will run in runspaces (threads)...
$ScriptBlock = {
    Param($IPAddress)
    Write-Host "Testing port 3180 on $IPAddress"

    try {
        $tcpClient = New-Object System.Net.Sockets.TcpClient
        if ($tcpClient.ConnectAsync($IPAddress, 3180).Wait(1000)) {
            $tcpClient.Close()
            return $IPAddress
        } else {
            return $null
        }
    }
    catch {
        # do nothing
        return $null
    }
}

$AvailableIPs = 0

foreach ($localIp in $localIps) {
    $Subnet = Get-IPv4Subnet -IPv4Address $localIp.IPAddress -CIDR $localIp.SubnetMask
    # Assign Start and End IPv4-Address
    $StartIPv4Address = $Subnet.NetworkID
    $EndIPv4Address = $Subnet.Broadcast

    # Convert Start and End IPv4-Address to Int64
    $StartIPv4Address_Int64 = (Convert-IPv4Address -IPv4Address $StartIPv4Address.ToString()).Int64
    $EndIPv4Address_Int64 = (Convert-IPv4Address -IPv4Address $EndIPv4Address.ToString()).Int64

    # Calculate IPs to scan (range)
    $IPsToScan = ($EndIPv4Address_Int64 - $StartIPv4Address_Int64)

    Write-Output "Scanning range from $StartIPv4Address($StartIPv4Address_Int64) to $EndIPv4Address($EndIPv4Address_Int64) ($($IPsToScan + 1) IPs)"
    
    # Create RunspacePool and Jobs
    $RunspacePool = [System.Management.Automation.Runspaces.RunspaceFactory]::CreateRunspacePool(1, 128)
    $RunspacePool.Open()
    [System.Collections.ArrayList]$Jobs = @()

    # Set up jobs for each IP...
    for ($i = $StartIPv4Address_Int64; $i -le $EndIPv4Address_Int64; $i++) { 
        # Convert IP back from Int64
        $IPv4Address = (Convert-IPv4Address -Int64 $i).IPv4Address                

        $PowerShell = [powershell]::Create()
        $PowerShell.RunspacePool = $RunspacePool
        $PowerShell.AddScript($ScriptBlock).AddArgument($IPv4Address) | Out-Null

        $JobObj = New-Object -TypeName PSObject -Property @{
            Result   = $PowerShell.BeginInvoke()
            PowerShell = $PowerShell
        }

        $Jobs.Add($JobObj) | Out-Null
    }
    Write-Output "Waiting for Scan ood to activate..."

    # Process results, while waiting for other jobs
    Do {
        $Jobs_ToProcess = $Jobs | Where-Object -FilterScript { $_.Result.IsCompleted }
        if ($null -eq $Jobs_ToProcess) {
            Start-Sleep 1
            continue
        }

        foreach ($Job in $Jobs_ToProcess) {
            $Job_Result = $Job.PowerShell.EndInvoke($Job.Result)
            # $Job.PowerShell.Dispose()
            $Jobs.Remove($Job)
            if ($Job_Result) {
                #Write-Output $Job_Result
                $AvailableIPs++
                Write-Output "Please open http://$($Job_Result):3180/index.html in your browser to activate the device"
                Start-Process "http://$($Job_Result):3180/index.html"
            }
        }
        
    } While ($Jobs.Count -gt 0)

    $RunspacePool.Close()
    $RunspacePool.Dispose()
}

if ($AvailableIPs -eq 0) {
    Write-Output "No devices found"
}