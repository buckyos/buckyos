
# Disable the backup_service service to start on boot
systemctl disable backup_service.service

# Stop the backup_service service
systemctl stop backup_service.service
