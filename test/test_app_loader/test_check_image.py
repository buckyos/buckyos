import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch, MagicMock
import importlib.util
import importlib.machinery


REPO_ROOT = Path(__file__).resolve().parents[2]
APP_LOADER_DIR = REPO_ROOT / "src" / "rootfs" / "bin" / "app-loader"


def _import_app_loader_module(module_name: str):
    """
    Import a module from `src/rootfs/bin/app-loader`.

    Note:
    - The directory name contains '-', so it cannot be imported as a Python package.
    - Some files are executable scripts without `.py` extension (e.g. `start`).
      For those, we load by file path.
    """
    py_path = APP_LOADER_DIR / f"{module_name}.py"
    plain_path = APP_LOADER_DIR / module_name

    if py_path.exists():
        sys.path.insert(0, str(APP_LOADER_DIR))
        return __import__(module_name)

    if plain_path.exists():
        loader = importlib.machinery.SourceFileLoader(f"app_loader_{module_name}", str(plain_path))
        spec = importlib.util.spec_from_loader(f"app_loader_{module_name}", loader)
        if spec is None or spec.loader is None:
            raise ImportError(f"Cannot load module from {plain_path}")
        mod = importlib.util.module_from_spec(spec)
        # Ensure dependencies like `util` can be imported.
        sys.path.insert(0, str(APP_LOADER_DIR))
        spec.loader.exec_module(mod)
        return mod

    raise ModuleNotFoundError(f"No module or script found for {module_name!r} in {APP_LOADER_DIR}")


class TestCheckImageExists(unittest.TestCase):
    """Test cases for check_image_exists function"""

    def setUp(self):
        self.start_mod = _import_app_loader_module("start")

    # def test_check_image_exists(self):
    #     """Test check_image_exists function"""
    #     result = self.start_mod.check_image_exists("buckyos/nightly-buckyos_filebrowser:0.5.1-amd64", "sha256:a3da360c99a263e8397aeb719d4bfb8756f557710333f7e8d497eea8290782c9")
    #     self.assertTrue(result)

    def test_image_not_exists(self):
        """Test when image does not exist"""
        with patch('os.popen') as mock_popen:
            mock_popen.return_value.read.return_value = ""
            result = self.start_mod.check_image_exists("nonexistent/image:latest")
            self.assertFalse(result)

    def test_image_exists_without_digest(self):
        """Test when image exists and no digest check is required"""
        with patch('os.popen') as mock_popen:
            mock_popen.return_value.read.return_value = "abc123"
            result = self.start_mod.check_image_exists("myimage:latest")
            self.assertTrue(result)

    def test_image_exists_with_matching_digest(self):
        """Test when image exists and digest matches"""
        image_name = "buckyos/nightly-buckyos_filebrowser:0.5.1-amd64"
        digest = "sha256:a3da360c99a263e8397aeb719d4bfb8756f557710333f7e8d497eea8290782c9"
        
        with patch('os.popen') as mock_popen:
            # Mock docker images -q command to return an image ID
            # Mock docker image inspect command to return RepoDigests
            def popen_side_effect(cmd):
                mock_result = MagicMock()
                if "docker images -q" in cmd:
                    mock_result.read.return_value = "abc123\n"
                elif "docker image inspect" in cmd:
                    # Simulate the RepoDigests returned by Docker
                    repo_digests = [
                        f"{image_name}@{digest}"
                    ]
                    mock_result.read.return_value = json.dumps(repo_digests)
                return mock_result
            
            mock_popen.side_effect = popen_side_effect
            result = self.start_mod.check_image_exists(image_name, digest)
            self.assertTrue(result)

    def test_image_exists_with_digest_prefix(self):
        """Test when digest is provided with image@ prefix"""
        image_name = "buckyos/nightly-buckyos_filebrowser:0.5.1-amd64"
        digest_with_prefix = f"{image_name}@sha256:a3da360c99a263e8397aeb719d4bfb8756f557710333f7e8d497eea8290782c9"
        digest_only = "sha256:a3da360c99a263e8397aeb719d4bfb8756f557710333f7e8d497eea8290782c9"
        
        with patch('os.popen') as mock_popen:
            def popen_side_effect(cmd):
                mock_result = MagicMock()
                if "docker images -q" in cmd:
                    mock_result.read.return_value = "abc123\n"
                elif "docker image inspect" in cmd:
                    repo_digests = [
                        f"{image_name}@{digest_only}"
                    ]
                    mock_result.read.return_value = json.dumps(repo_digests)
                return mock_result
            
            mock_popen.side_effect = popen_side_effect
            result = self.start_mod.check_image_exists(image_name, digest_with_prefix)
            self.assertTrue(result)

    def test_image_exists_with_mismatched_digest(self):
        """Test when image exists but digest does not match"""
        image_name = "myimage:latest"
        expected_digest = "sha256:expected123"
        actual_digest = "sha256:actual456"
        
        with patch('os.popen') as mock_popen:
            def popen_side_effect(cmd):
                mock_result = MagicMock()
                if "docker images -q" in cmd:
                    mock_result.read.return_value = "abc123\n"
                elif "docker image inspect" in cmd:
                    repo_digests = [
                        f"{image_name}@{actual_digest}"
                    ]
                    mock_result.read.return_value = json.dumps(repo_digests)
                return mock_result
            
            mock_popen.side_effect = popen_side_effect
            result = self.start_mod.check_image_exists(image_name, expected_digest)
            self.assertFalse(result)

    def test_image_inspect_fails(self):
        """Test when docker inspect command fails or returns invalid JSON"""
        with patch('os.popen') as mock_popen:
            def popen_side_effect(cmd):
                mock_result = MagicMock()
                if "docker images -q" in cmd:
                    mock_result.read.return_value = "abc123\n"
                elif "docker image inspect" in cmd:
                    mock_result.read.return_value = ""
                return mock_result
            
            mock_popen.side_effect = popen_side_effect
            result = self.start_mod.check_image_exists("myimage:latest", "sha256:abc123")
            self.assertFalse(result)

    def test_image_inspect_invalid_json(self):
        """Test when docker inspect returns invalid JSON"""
        with patch('os.popen') as mock_popen:
            def popen_side_effect(cmd):
                mock_result = MagicMock()
                if "docker images -q" in cmd:
                    mock_result.read.return_value = "abc123\n"
                elif "docker image inspect" in cmd:
                    mock_result.read.return_value = "invalid json"
                return mock_result
            
            mock_popen.side_effect = popen_side_effect
            result = self.start_mod.check_image_exists("myimage:latest", "sha256:abc123")
            self.assertFalse(result)

    def test_image_with_multiple_repo_digests(self):
        """Test when image has multiple RepoDigests (multiple tags)"""
        image_name = "myimage:latest"
        digest = "sha256:abc123"
        
        with patch('os.popen') as mock_popen:
            def popen_side_effect(cmd):
                mock_result = MagicMock()
                if "docker images -q" in cmd:
                    mock_result.read.return_value = "abc123\n"
                elif "docker image inspect" in cmd:
                    # Image might have multiple RepoDigests
                    repo_digests = [
                        "myrepo/myimage:v1@sha256:different123",
                        f"{image_name}@{digest}",
                        "myrepo/myimage:v2@sha256:another456"
                    ]
                    mock_result.read.return_value = json.dumps(repo_digests)
                return mock_result
            
            mock_popen.side_effect = popen_side_effect
            result = self.start_mod.check_image_exists(image_name, digest)
            self.assertTrue(result)

    def test_normalize_digest_function(self):
        """Test the _normalize_digest helper function"""
        normalize_digest = self.start_mod._normalize_digest
        
        # Test with plain digest
        self.assertEqual(
            normalize_digest("sha256:abc123"),
            "sha256:abc123"
        )
        
        # Test with image@digest format
        self.assertEqual(
            normalize_digest("myimage:tag@sha256:abc123"),
            "sha256:abc123"
        )
        
        # Test with None
        self.assertIsNone(normalize_digest(None))
        
        # Test with empty string
        self.assertIsNone(normalize_digest(""))
        
        # Test with whitespace
        self.assertIsNone(normalize_digest("  "))

    def test_image_pulled_by_digest_with_empty_repo_digests(self):
        """
        Test the real-world scenario: when image is pulled by digest and then tagged,
        the new tag might not have RepoDigests populated.
        
        This simulates the bug reported:
        - docker pull image:tag@digest creates an image
        - docker tag is used to create a local tag
        - The locally tagged image might have empty RepoDigests
        
        After the fix, check_image_exists should fall back to checking the image Id.
        """
        image_name = "buckyos/nightly-buckyos_filebrowser:0.5.1-amd64"
        digest = "sha256:a3da360c99a263e8397aeb719d4bfb8756f557710333f7e8d497eea8290782c9"
        
        with patch('os.popen') as mock_popen:
            def popen_side_effect(cmd):
                mock_result = MagicMock()
                if "docker images -q" in cmd:
                    # Image exists locally
                    mock_result.read.return_value = "abc123\n"
                elif "docker image inspect" in cmd:
                    if "RepoDigests" in cmd:
                        # RepoDigests is empty because the image was tagged locally
                        repo_digests = []
                        mock_result.read.return_value = json.dumps(repo_digests)
                    elif ".Id" in cmd:
                        # But the image Id matches the digest
                        mock_result.read.return_value = digest
                return mock_result
            
            mock_popen.side_effect = popen_side_effect
            result = self.start_mod.check_image_exists(image_name, digest)
            # After fix: should return True by checking image Id
            self.assertTrue(result)

    def test_image_pulled_by_digest_without_matching_repo(self):
        """
        Test scenario where image is pulled from registry A but the RepoDigest
        references a different repository name.
        
        For example:
        - Pull: docker.io/repo/image:tag@sha256:xxx
        - RepoDigests might be: ["docker.io/repo/image@sha256:xxx"]
        - But we check against: local/image:tag
        """
        local_image_name = "myimage:latest"
        digest = "sha256:abc123"
        registry_image_name = "docker.io/library/myimage"
        
        with patch('os.popen') as mock_popen:
            def popen_side_effect(cmd):
                mock_result = MagicMock()
                if "docker images -q" in cmd:
                    mock_result.read.return_value = "abc123\n"
                elif "docker image inspect" in cmd:
                    # RepoDigests contains the full registry path
                    repo_digests = [
                        f"{registry_image_name}@{digest}"
                    ]
                    mock_result.read.return_value = json.dumps(repo_digests)
                return mock_result
            
            mock_popen.side_effect = popen_side_effect
            result = self.start_mod.check_image_exists(local_image_name, digest)
            # The digest matches, so it should return True
            self.assertTrue(result)


if __name__ == "__main__":
    unittest.main(verbosity=2)
