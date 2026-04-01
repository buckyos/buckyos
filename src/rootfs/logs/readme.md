# $BUCKYOS_ROOT/logs

Storage Raw logs from modules.

edit $BUCKYOS_ROOT/logs/log_settings.cfg for dynamic control logger.
```toml
[default]
level = "warn"
size = "128MB"
count = 3

[system_config]
level = "debug"
max_file_size = "64MB"
```