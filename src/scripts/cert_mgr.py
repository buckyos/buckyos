"""
Certificate management module: Generate and manage CA certificates and CA-signed certificates
for test environments using openssl.
"""
import os
import subprocess
import tempfile
import argparse
from pathlib import Path


class CertManager:
    """Certificate manager using openssl command-line tools"""
    
    def __init__(self, openssl_path="openssl"):
        """
        Initialize certificate manager
        
        Args:
            openssl_path: Path to openssl command, default is "openssl"
        """
        self.openssl_path = openssl_path
        self._check_openssl()
    
    def _check_openssl(self):
        """Check if openssl is available"""
        try:
            result = subprocess.run(
                [self.openssl_path, "version"],
                capture_output=True,
                text=True,
                check=True
            )
            print(f"Using OpenSSL: {result.stdout.strip()}")
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            raise RuntimeError(f"OpenSSL not found or not working: {e}")
    
    def create_ca(
        self,
        output_dir: str,
        name: str = "devtest"
    ):
        """
        Create test CA certificate
        
        Args:
            output_dir: Output directory
            name: CA name (used as Common Name)
        
        Returns:
            Tuple of (ca_cert_path, ca_key_path)
        """
        output_path = Path(output_dir)
        output_path.mkdir(parents=True, exist_ok=True)
        
        ca_key_path = output_path / f"{name}_ca_key.pem"
        ca_cert_path = output_path / f"{name}_ca_cert.pem"
        
        # Auto-construct other fields
        common_name = name
        organization = f"{name}'s Dev Test Environment"
        validity_days = 3650  # 10 years
        key_size = 4096  # 4096-bit key
        
        print(f"Creating test CA certificate...")
        print(f"  Name: {common_name}")
        print(f"  Organization: {organization}")
        print(f"  Validity: {validity_days} days")
        print(f"  Key Size: {key_size} bits")
        
        # Generate CA private key
        print(f"Generating CA private key...")
        subprocess.run(
            [
                self.openssl_path, "genrsa",
                "-out", str(ca_key_path),
                str(key_size)
            ],
            check=True
        )
        
        # Generate self-signed CA certificate
        print(f"Generating CA certificate...")
        subprocess.run(
            [
                self.openssl_path, "req",
                "-x509", "-new", "-nodes",
                "-key", str(ca_key_path),
                "-sha256",
                "-days", str(validity_days),
                "-out", str(ca_cert_path),
                "-subj", f"/C=US/ST=California/L=San Jose/O={organization}/OU=Test/CN={common_name}"
            ],
            check=True
        )
        
        print(f"CA certificate created successfully!")
        print(f"  Certificate: {ca_cert_path}")
        print(f"  Private Key: {ca_key_path}")
        
        return str(ca_cert_path), str(ca_key_path)
    
    def create_cert_from_ca(
        self,
        ca_dir: str,
        hostname: str,
        target_dir: str,
        hostnames=None,
    ):
        """
        Generate server certificate from CA
        
        Args:
            ca_dir: Directory containing CA certificate and key
            hostname: Hostname (supports wildcards, e.g., *.example.com)
            target_dir: Output directory
        
        Returns:
            Tuple of (cert_path, key_path)
        """
        # Get certificate and key paths from CA directory
        # Actively search for *_ca_cert.pem pattern
        ca_dir_path = Path(ca_dir)
        
        # Find CA certificate file matching *_ca_cert.pem pattern
        ca_cert_files = list(ca_dir_path.glob("*_ca_cert.pem"))
        if not ca_cert_files:
            raise FileNotFoundError(f"No CA certificate found matching *_ca_cert.pem pattern in {ca_dir}")
        if len(ca_cert_files) > 1:
            raise FileNotFoundError(f"Multiple CA certificates found in {ca_dir}: {[f.name for f in ca_cert_files]}. Please specify which one to use.")
        
        ca_cert_path = ca_cert_files[0]
        
        # Derive key filename from certificate filename
        # e.g., devtest_ca_cert.pem -> devtest_ca_key.pem
        ca_key_filename = ca_cert_path.name.replace("_ca_cert.pem", "_ca_key.pem")
        ca_key_path = ca_dir_path / ca_key_filename
        
        if not ca_key_path.exists():
            raise FileNotFoundError(f"CA key not found: {ca_key_path} (expected based on certificate {ca_cert_path.name})")
        
        # Auto-construct parameters
        dns_names = hostnames or [hostname]
        common_name = dns_names[0]
        ip_addresses = None
        validity_days = 365  # 1 year
        key_size = 2048  # 2048-bit key
        
        # Auto-generate output filenames (based on primary hostname, replace special characters)
        safe_hostname = common_name.replace("*", "wildcard").replace(".", "_")
        output_dir = Path(target_dir)
        output_dir.mkdir(parents=True, exist_ok=True)
        output_cert = output_dir / f"{safe_hostname}.crt"
        output_key = output_dir / f"{safe_hostname}.key"
        
        print(f"Creating certificate from CA...")
        print(f"  Hostname: {hostname}")
        print(f"  Common Name: {common_name}")
        print(f"  DNS Names: {', '.join(dns_names)}")
        print(f"  Validity: {validity_days} days")
        print(f"  Key Size: {key_size} bits")
        
        # Generate server private key
        print(f"Generating server private key...")
        subprocess.run(
            [
                self.openssl_path, "genrsa",
                "-out", str(output_key),
                str(key_size)
            ],
            check=True
        )
        
        # Create temporary config file for Subject Alternative Names
        with tempfile.NamedTemporaryFile(mode='w', suffix='.conf', delete=False) as conf_file:
            conf_path = conf_file.name
            conf_file.write("[req]\n")
            conf_file.write("distinguished_name = req_distinguished_name\n")
            conf_file.write("req_extensions = v3_req\n")
            conf_file.write("\n[req_distinguished_name]\n")
            conf_file.write(f"CN = {common_name}\n")
            conf_file.write("\n[v3_req]\n")
            conf_file.write("keyUsage = keyEncipherment, dataEncipherment\n")
            conf_file.write("extendedKeyUsage = serverAuth\n")
            
            # Add Subject Alternative Names
            if dns_names or ip_addresses:
                conf_file.write("subjectAltName = @alt_names\n")
                conf_file.write("\n[alt_names]\n")
                dns_index = 1
                ip_index = 1
                for dns in (dns_names or []):
                    conf_file.write(f"DNS.{dns_index} = {dns}\n")
                    dns_index += 1
                for ip in (ip_addresses or []):
                    conf_file.write(f"IP.{ip_index} = {ip}\n")
                    ip_index += 1
        
        try:
            # Generate certificate signing request (CSR)
            print(f"Generating certificate signing request...")
            csr_path = str(output_key).replace('.pem', '.csr').replace('.key', '.csr')
            subprocess.run(
                [
                    self.openssl_path, "req",
                    "-new",
                    "-key", str(output_key),
                    "-out", csr_path,
                    "-config", conf_path,
                    "-subj", f"/CN={common_name}"
                ],
                check=True
            )
            
            # Sign certificate with CA
            print(f"Signing certificate with CA...")
            subprocess.run(
                [
                    self.openssl_path, "x509",
                    "-req",
                    "-in", csr_path,
                    "-CA", str(ca_cert_path),
                    "-CAkey", str(ca_key_path),
                    "-CAcreateserial",
                    "-out", str(output_cert),
                    "-days", str(validity_days),
                    "-sha256",
                    "-extensions", "v3_req",
                    "-extfile", conf_path
                ],
                check=True
            )
            
            # Clean up temporary files
            os.unlink(csr_path)
            serial_file = ca_cert_path.parent / f"{ca_cert_path.stem}.srl"
            if serial_file.exists():
                os.unlink(serial_file)
        finally:
            # Clean up config file
            if os.path.exists(conf_path):
                os.unlink(conf_path)
        
        print(f"Certificate created successfully!")
        print(f"  Certificate: {output_cert}")
        print(f"  Private Key: {output_key}")
        
        return str(output_cert), str(output_key)
    
    def install_ca_to_system(self, ca_dir: str, use_sudo: bool = True):
        """
        Install CA certificate to system trust store
        
        Note: Most operating systems can automatically install CA certificates via command line,
        but administrator privileges are required. For test environments (usually Linux VMs),
        this can be fully automated.
        
        Args:
            ca_dir: Directory containing CA certificate and key
            use_sudo: Whether to use sudo (Linux/macOS), default is True
        """
        import platform
        
        # Get certificate path from CA directory
        # Actively search for *_ca_cert.pem pattern
        ca_dir_path = Path(ca_dir)
        
        # Find CA certificate file matching *_ca_cert.pem pattern
        ca_cert_files = list(ca_dir_path.glob("*_ca_cert.pem"))
        if not ca_cert_files:
            raise FileNotFoundError(f"No CA certificate found matching *_ca_cert.pem pattern in {ca_dir}")
        if len(ca_cert_files) > 1:
            raise FileNotFoundError(f"Multiple CA certificates found in {ca_dir}: {[f.name for f in ca_cert_files]}. Please specify which one to use.")
        
        ca_cert_path = ca_cert_files[0]
        
        system = platform.system()
        print(f"Installing CA certificate to system trust store ({system})...")
        print(f"  CA Certificate: {ca_cert_path}")
        
        if system == "Linux":
            # Linux systems - can be automatically installed via command line
            # Detect Linux distribution
            try:
                with open("/etc/os-release", "r") as f:
                    os_release = f.read()
                    if "debian" in os_release.lower() or "ubuntu" in os_release.lower():
                        # Debian/Ubuntu systems
                        target_dir = Path("/usr/local/share/ca-certificates")
                        target_cert = target_dir / "buckyos-test-ca.crt"
                        
                        print(f"Copying CA certificate to {target_dir}...")
                        cmd = ["sudo", "cp", ca_cert_path, str(target_cert)] if use_sudo else ["cp", ca_cert_path, str(target_cert)]
                        subprocess.run(cmd, check=True)
                        
                        print("Updating CA certificate store...")
                        cmd = ["sudo", "update-ca-certificates"] if use_sudo else ["update-ca-certificates"]
                        subprocess.run(cmd, check=True)
                        
                        print("CA certificate installed successfully!")
                    elif "redhat" in os_release.lower() or "centos" in os_release.lower() or "fedora" in os_release.lower():
                        # RHEL/CentOS/Fedora systems
                        target_dir = Path("/etc/pki/ca-trust/source/anchors")
                        target_cert = target_dir / "buckyos-test-ca.crt"
                        
                        print(f"Copying CA certificate to {target_dir}...")
                        cmd = ["sudo", "cp", ca_cert_path, str(target_cert)] if use_sudo else ["cp", ca_cert_path, str(target_cert)]
                        subprocess.run(cmd, check=True)
                        
                        print("Updating CA certificate store...")
                        cmd = ["sudo", "update-ca-trust"] if use_sudo else ["update-ca-trust"]
                        subprocess.run(cmd, check=True)
                        
                        print("CA certificate installed successfully!")
                    else:
                        # Other Linux distributions, try generic method
                        target_dir = Path("/usr/local/share/ca-certificates")
                        target_cert = target_dir / "buckyos-test-ca.crt"
                        
                        print(f"Copying CA certificate to {target_dir}...")
                        cmd = ["sudo", "cp", ca_cert_path, str(target_cert)] if use_sudo else ["cp", ca_cert_path, str(target_cert)]
                        subprocess.run(cmd, check=True)
                        
                        # Try to update certificate store
                        for update_cmd in [["sudo", "update-ca-certificates"], ["sudo", "update-ca-trust"]]:
                            try:
                                if not use_sudo:
                                    update_cmd = update_cmd[1:]  # Remove sudo
                                subprocess.run(update_cmd, check=True, capture_output=True)
                                print("CA certificate installed successfully!")
                                break
                            except (subprocess.CalledProcessError, FileNotFoundError):
                                continue
                        else:
                            print("Warning: Could not update CA certificate store automatically.")
                            print("You may need to run 'update-ca-certificates' or 'update-ca-trust' manually.")
            except FileNotFoundError:
                # Cannot detect distribution, use default method
                target_dir = Path("/usr/local/share/ca-certificates")
                target_cert = target_dir / "buckyos-test-ca.crt"
                
                print(f"Copying CA certificate to {target_dir}...")
                cmd = ["sudo", "cp", ca_cert_path, str(target_cert)] if use_sudo else ["cp", ca_cert_path, str(target_cert)]
                subprocess.run(cmd, check=True)
                
                print("CA certificate installed successfully!")
                print("Note: You may need to run 'update-ca-certificates' manually.")
        
        elif system == "Darwin":  # macOS
            # macOS - can be automatically installed via security command
            print("Installing CA certificate to macOS system keychain...")
            cmd = [
                "sudo", "security", "add-trusted-cert",
                "-d", "-r", "trustRoot",
                "-k", "/Library/Keychains/System.keychain",
                ca_cert_path
            ] if use_sudo else [
                "security", "add-trusted-cert",
                "-d", "-r", "trustRoot",
                "-k", "/Library/Keychains/System.keychain",
                ca_cert_path
            ]
            subprocess.run(cmd, check=True)
            print("CA certificate installed successfully!")
            print("Note: You may need to restart applications for the changes to take effect.")
        
        elif system == "Windows":
            # Windows - can be automatically installed via certutil command (requires administrator privileges)
            print("Installing CA certificate to Windows Trusted Root Certification Authorities...")
            print("Note: This requires administrator privileges.")
            cmd = ["certutil", "-addstore", "-f", "ROOT", ca_cert_path]
            subprocess.run(cmd, check=True)
            print("CA certificate installed successfully!")
            print("Note: You may need to restart applications for the changes to take effect.")
        
        else:
            print(f"Automatic CA certificate installation is not supported for {system}.")
            print(f"Please install the CA certificate manually:")
            print(f"  Certificate file: {ca_cert_path}")
            print("\nManual installation instructions:")
            if system == "Linux":
                print("  1. Copy the certificate to /usr/local/share/ca-certificates/")
                print("  2. Run: sudo update-ca-certificates")
            elif system == "Darwin":
                print("  1. Open Keychain Access")
                print("  2. Drag the certificate to System keychain")
                print("  3. Double-click and set trust to 'Always Trust'")
            elif system == "Windows":
                print("  1. Double-click the certificate file")
                print("  2. Click 'Install Certificate'")
                print("  3. Select 'Local Machine' and 'Place all certificates in the following store'")
                print("  4. Select 'Trusted Root Certification Authorities'")


def main():
    """Command-line entry point"""
    parser = argparse.ArgumentParser(description="BuckyOS Test Certificate Manager")
    subparsers = parser.add_subparsers(dest="command", help="Command to execute")
    
    # create_ca command
    create_ca_parser = subparsers.add_parser("create_ca", help="Create test CA certificate")
    create_ca_parser.add_argument("--target", required=True, help="Output directory")
    create_ca_parser.add_argument("--name", default="devtest", help="CA name (used as Common Name)")
    
    # create_cert command
    create_cert_parser = subparsers.add_parser("create_cert", help="Create certificate from CA")
    create_cert_parser.add_argument("--ca", required=True, help="CA certificate and key directory")
    create_cert_parser.add_argument("--hostname", required=True, help="Hostname (supports wildcards, e.g., *.example.com)")
    create_cert_parser.add_argument("--target", required=True, help="Output directory")
    
    # install_ca command
    install_ca_parser = subparsers.add_parser("install_ca", help="Install CA certificate to system")
    install_ca_parser.add_argument("--ca", required=True, help="CA certificate and key directory")
    
    args = parser.parse_args()
    
    if not args.command:
        parser.print_help()
        return
    
    cert_mgr = CertManager()
    
    if args.command == "create_ca":
        cert_mgr.create_ca(
            args.target,
            args.name
        )
    elif args.command == "create_cert":
        cert_mgr.create_cert_from_ca(
            args.ca,
            args.hostname,
            args.target
        )
    elif args.command == "install_ca":
        cert_mgr.install_ca_to_system(args.ca)


if __name__ == "__main__":
    main()

