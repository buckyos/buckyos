
# Enable the backup_service service to start on boot
systemctl enable backup_service.service

# Start the backup_service service
systemctl start backup_service.service
