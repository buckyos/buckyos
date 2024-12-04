import os
import sys
import time
import random
import multiprocessing as mp
import gc
from concurrent.futures import ProcessPoolExecutor

# 已完成的文件数
completed_files = mp.Value('i', 0)

def generate_random_file(file_path, size):
    with open(file_path, 'wb') as f:
        f.write(os.urandom(size))
    with completed_files.get_lock():
        completed_files.value += 1

def read_file(file_path):
    with open(file_path, 'rb') as f:
        f.read()
    with completed_files.get_lock():
        completed_files.value += 1

def print_progress(start_time, total_files, file_size, mode="写入"):
    while completed_files.value < total_files:
        elapsed = time.time() - start_time
        bytes_processed = completed_files.value * file_size
        speed = bytes_processed / elapsed if elapsed > 0 else 0
        print(f"\r{mode}进度: {completed_files.value}/{total_files} 文件 "
              f"已耗时: {elapsed:.1f}秒 "
              f"平均速度: {speed/1024/1024:.2f} MB/s", end="")
        time.sleep(1)

def gc_process():
    while True:
        gc.collect()
        time.sleep(1)

def main():
    if len(sys.argv) != 6:
        print("用法: python script.py <进程数> <单文件大小(字节)> <总文件数> <目标目录> <测试模式>")
        print("测试模式: write(只写入), read(只读取), both(读写都执行)")
        sys.exit(1)
        
    num_processes = int(sys.argv[1])
    file_size = int(sys.argv[2]) 
    total_files = int(sys.argv[3])
    target_dir = sys.argv[4]
    test_mode = sys.argv[5].lower()
    
    if test_mode not in ['write', 'read', 'both']:
        print("测试模式必须是 write, read 或 both 之一")
        sys.exit(1)
    
    if not os.path.exists(target_dir):
        os.makedirs(target_dir)
        
    print(f"开始测试...")
    print(f"进程数: {num_processes}")
    print(f"单文件大小: {file_size} 字节")
    print(f"总文件数: {total_files}")
    print(f"目标目录: {target_dir}")
    print(f"测试模式: {test_mode}")
    
    total_bytes = total_files * file_size

    # 启动GC进程
    gc_proc = mp.Process(target=gc_process)
    gc_proc.daemon = True
    gc_proc.start()

    # 写入测试
    if test_mode in ['write', 'both']:
        print(f"\n开始写入文件...")
        completed_files.value = 0
        start_time = time.time()
        
        progress_proc = mp.Process(target=print_progress, 
                                 args=(start_time, total_files, file_size, "写入"))
        progress_proc.daemon = True
        progress_proc.start()
        
        with ProcessPoolExecutor(max_workers=num_processes) as executor:
            futures = []
            for i in range(total_files):
                file_path = os.path.join(target_dir, f"{i}")
                futures.append(executor.submit(generate_random_file, file_path, file_size))
                
            for future in futures:
                future.result()
                
        end_time = time.time()
        total_write_time = end_time - start_time
        
        print(f"\n\n完成写入文件")
        print(f"写入总耗时: {total_write_time:.2f} 秒")
        print(f"写入平均速度: {total_bytes/total_write_time/1024/1024:.2f} MB/s")

    # 读取测试
    if test_mode in ['read', 'both']:
        print(f"\n开始读取文件...")
        completed_files.value = 0
        start_time = time.time()

        progress_proc = mp.Process(target=print_progress, 
                                 args=(start_time, total_files, file_size, "读取"))
        progress_proc.daemon = True
        progress_proc.start()

        with ProcessPoolExecutor(max_workers=num_processes) as executor:
            futures = []
            for i in range(total_files):
                file_path = os.path.join(target_dir, f"{i}")
                futures.append(executor.submit(read_file, file_path))
                
            for future in futures:
                future.result()

        end_time = time.time()
        total_read_time = end_time - start_time
        
        print(f"\n\n完成读取文件")
        print(f"读取总耗时: {total_read_time:.2f} 秒")
        print(f"读取平均速度: {total_bytes/total_read_time/1024/1024:.2f} MB/s")
    
if __name__ == "__main__":
    main()
