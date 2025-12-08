from pathlib import Path
import json

class AppConfig:
    """应用配置"""
    def __init__(self, app_name: str):
        self.app_name = app_name
        self.commands : dict [str, list[str]]= None
        self.directories : dict [str, str] = None
        self.version : str = None

    def load_from_file(self, file_path: Path):
        with open(file_path, 'r') as f:
            app_config = json.load(f)
        app_name = app_config.get("name")
        if app_name is None:
            raise ValueError(f"App name not found in {file_path}")
        if self.app_name != app_name:
            raise ValueError(f"App name mismatch in {file_path}")

        self.version = app_config.get("version")
        self.commands = app_config.get("commands")
        self.directories = app_config.get("directories")

    def get_command(self, cmd_name: str) -> list[str]:
        return self.commands.get(cmd_name)

    def get_dir(self, dir_name: str) -> str:
        return self.directories.get(dir_name)

class AppList:
    """应用列表"""
    def __init__(self, app_dir: Path):
        self.app_dir: Path = app_dir
        self.app_list: dict [str, AppConfig]= {}

    def load_app_list(self):
        # 打开app_dir下的所有json文件，并加载到app_list中
        for file in self.app_dir.glob('*.json'):
            app_config = AppConfig(file.stem)
            app_config.load_from_file(file)
            self.app_list[app_config.app_name] = app_config

    def get_app(self, app_name: str) -> AppConfig:
        return self.app_list.get(app_name)
    
    def get_all_app_names(self) -> list[str]:
        return list(self.app_list.keys())