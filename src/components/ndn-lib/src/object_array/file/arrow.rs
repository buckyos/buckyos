use super::super::memory_cache::ObjectArrayMemoryCache;
use super::super::storage::{
    ObjectArrayCacheType, ObjectArrayInnerCache, ObjectArrayStorageType,
    ObjectArrayStorageWriter,
};
use crate::ObjId;
use crate::{NdnError, NdnResult};
use arrow::array::{
    ArrayBuilder, ArrayRef, BinaryArray, BinaryBuilder, StringArray, StringBuilder,
};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::reader::FileReader;
use arrow::ipc::writer::FileWriter;
use arrow::record_batch::RecordBatch;
use buckyos_kit::init_logging;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct ObjectArrayArrowCache {
    schema: Arc<Schema>,
    batch: RecordBatch,
}

impl ObjectArrayArrowCache {
    pub fn new_empty() -> Self {
        let schema = Schema::new(vec![
            Field::new("obj_type", DataType::Utf8, false),
            Field::new("obj_hash", DataType::Binary, false),
        ]);
        let schema = Arc::new(schema);
        let batch = RecordBatch::new_empty(schema.clone());

        Self { schema, batch }
    }

    pub fn new(schema: Arc<Schema>, batch: RecordBatch) -> Self {
        Self { schema, batch }
    }

    fn get(&self, index: usize) -> NdnResult<Option<ObjId>> {
        if index >= self.len() {
            let msg = format!("Index out of bounds: {} >= {}", index, self.len());
            error!("{}", msg);
            return Err(NdnError::OffsetTooLarge(msg));
        }

        let obj_type_array = self
            .batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let obj_hash_array = self
            .batch
            .column(1)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();

        let obj_type = obj_type_array.value(index).to_string();
        let obj_hash = obj_hash_array.value(index);

        Ok(Some(ObjId::new_by_raw(obj_type, obj_hash.to_vec())))
    }

    fn get_range(&self, start: usize, end: usize) -> NdnResult<Vec<ObjId>> {
        if start >= self.len() || end > self.len() || start > end {
            let msg = format!(
                "Index out of bounds: {} >= {} or {} > {}",
                start,
                self.len(),
                end,
                self.len()
            );
            error!("{}", msg);
            return Err(NdnError::OffsetTooLarge(msg));
        }

        let obj_type_array = self
            .batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let obj_hash_array = self
            .batch
            .column(1)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let mut ret = Vec::with_capacity(end - start);

        for index in start..end {
            let obj_type = obj_type_array.value(index).to_string();
            let obj_hash = obj_hash_array.value(index);
            ret.push(ObjId::new_by_raw(obj_type, obj_hash.to_vec()));
        }

        Ok(ret)
    }

    fn len(&self) -> usize {
        self.batch.num_rows()
    }

    pub fn into_memory_cache(self) -> Box<dyn ObjectArrayInnerCache> {
        let obj_type_array = self
            .batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let obj_hash_array = self
            .batch
            .column(1)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();

        let mut cache = Vec::with_capacity(self.len());
        for index in 0..self.len() {
            let obj_type = obj_type_array.value(index).to_string();
            let obj_hash = obj_hash_array.value(index);
            cache.push(ObjId::new_by_raw(obj_type, obj_hash.to_vec()));
        }

        Box::new(ObjectArrayMemoryCache::new_array(cache))
    }
}

#[async_trait::async_trait]
impl ObjectArrayInnerCache for ObjectArrayArrowCache {
    fn get_type(&self) -> ObjectArrayCacheType {
        ObjectArrayCacheType::Arrow
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn is_readonly(&self) -> bool {
        true
    }

    fn clone_cache(&self, read_only: bool) -> NdnResult<Box<dyn ObjectArrayInnerCache>> {
        let s = self.clone();
        if read_only {
            return Ok(Box::new(s));
        }

        let ret = s.into_memory_cache();
        Ok(ret)
    }

    fn get(&self, index: usize) -> NdnResult<Option<ObjId>> {
        self.get(index)
    }

    fn get_range(&self, start: usize, end: usize) -> NdnResult<Vec<ObjId>> {
        self.get_range(start, end)
    }

    fn append(&mut self, _value: &ObjId) -> NdnResult<()> {
        let msg = "Append is not supported in Arrow cache".to_string();
        error!("{}", msg);
        Err(NdnError::InvalidState(msg))
    }

    fn insert(&mut self, _index: usize, _value: &ObjId) -> NdnResult<()> {
        let msg = "Insert is not supported in Arrow cache".to_string();
        error!("{}", msg);
        Err(NdnError::InvalidState(msg))
    }

    fn remove(&mut self, _index: usize) -> NdnResult<Option<ObjId>> {
        let msg = "Remove is not supported in Arrow cache".to_string();
        error!("{}", msg);
        Err(NdnError::InvalidState(msg))
    }

    fn clear(&mut self) -> NdnResult<()> {
        let msg = "Clear is not supported in Arrow cache".to_string();
        error!("{}", msg);
        Err(NdnError::InvalidState(msg))
    }

    fn pop(&mut self) -> NdnResult<Option<ObjId>> {
        let msg = "Pop is not supported in Arrow cache".to_string();
        error!("{}", msg);
        Err(NdnError::InvalidState(msg))
    }
}

pub struct ObjectArrayArrowWriter {
    file_path: PathBuf,
    schema: Arc<Schema>,

    obj_type_builder: StringBuilder,
    obj_hash_builder: BinaryBuilder,
}

impl ObjectArrayArrowWriter {
    pub fn new(file_path: PathBuf, len: Option<usize>) -> Self {
        let mut obj_type_builder;
        let mut obj_hash_builder;

        match len {
            Some(len) => {
                obj_type_builder = StringBuilder::with_capacity(len, len * 40);
                obj_hash_builder = BinaryBuilder::with_capacity(len, len * 40);
            }
            None => {
                obj_type_builder = StringBuilder::new();
                obj_hash_builder = BinaryBuilder::new();
            }
        }

        let schema = Schema::new(vec![
            Field::new("obj_type", DataType::Utf8, false),
            Field::new("obj_hash", DataType::Binary, false),
        ]);

        Self {
            file_path,
            schema: Arc::new(schema),
            obj_type_builder,
            obj_hash_builder,
        }
    }

    async fn append(&mut self, value: &ObjId) -> NdnResult<()> {
        self.obj_type_builder.append_value(&value.obj_type);
        self.obj_hash_builder.append_value(&value.obj_hash);

        Ok(())
    }

    async fn len(&self) -> NdnResult<usize> {
        Ok(self.obj_type_builder.len())
    }

    async fn flush(&mut self) -> NdnResult<()> {
        let obj_type_array: ArrayRef = Arc::new(self.obj_type_builder.finish());
        let obj_hash_array: ArrayRef = Arc::new(self.obj_hash_builder.finish());

        let batch = RecordBatch::try_new(self.schema.clone(), vec![obj_type_array, obj_hash_array])
            .map_err(|e| {
                let msg = format!("Failed to create record batch: {}", e);
                error!("{}", msg);
                NdnError::InvalidData(msg)
            })?;

        let file = File::create(&self.file_path).map_err(|e| {
            let msg = format!("Failed to create file: {:?}, {}", self.file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let mut writer = FileWriter::try_new(file, &self.schema).map_err(|e| {
            let msg = format!("Failed to create file writer: {:?}, {}", self.file_path, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        writer.write(&batch).map_err(|e| {
            let msg = format!("Failed to write record batch: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        writer.finish().map_err(|e| {
            let msg = format!("Failed to finish file writer: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        info!("ObjectArray's arrow file written to: {:?}", self.file_path);

        Ok(())
    }
}

#[async_trait::async_trait]
impl ObjectArrayStorageWriter for ObjectArrayArrowWriter {
    async fn file_path(&self) -> NdnResult<PathBuf> {
        Ok(self.file_path.clone())
    }

    async fn append(&mut self, value: &ObjId) -> NdnResult<()> {
        self.append(value).await
    }

    async fn len(&self) -> NdnResult<usize> {
        self.len().await
    }

    async fn flush(&mut self) -> NdnResult<()> {
        self.flush().await
    }
}

pub struct ObjectArrayArrowReader {
    cache: Box<dyn ObjectArrayInnerCache>,
}

impl ObjectArrayArrowReader {
    pub fn new(cache: Box<dyn ObjectArrayInnerCache>) -> Self {
        Self { cache }
    }

    pub fn into_cache(self) -> Box<dyn ObjectArrayInnerCache> {
        self.cache
    }

    pub async fn open(file: &Path, readonly: bool) -> NdnResult<Self> {
        let f = File::open(&file).map_err(|e| {
            let msg = format!("Failed to open file: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let mut reader = FileReader::try_new(f, None).map_err(|e| {
            let msg = format!("Failed to create file reader: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        // Get the first record batch
        if reader.num_batches() == 0 {
            let msg = format!("No record batch found in file: {:?}", file);
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        let batch = reader.next().unwrap().map_err(|e| {
            let msg = format!("Failed to read record batch: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let schema = batch.schema().clone();
        let len = batch.num_rows();

        let cache = ObjectArrayArrowCache::new(schema, batch);
        let cache: Box<dyn ObjectArrayInnerCache> = if readonly {
            Box::new(cache)
        } else {
            // If not readonly, convert to memory cache
            cache.into_memory_cache()
        };

        let ret = Self::new(cache);
        Ok(ret)
    }
}