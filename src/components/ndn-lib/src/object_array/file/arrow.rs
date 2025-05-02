use arrow::array::{ArrayBuilder, ArrayRef, BinaryArray, BinaryBuilder, StringArray, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use arrow::ipc::writer::FileWriter;
use arrow::ipc::reader::FileReader;
use std::path::Path;
use std::sync::Arc;
use crate::ObjId;
use crate::{
    NdnError,
    NdnResult,
};
use std::fs::File;

pub struct ObjectArrayArrowWriter {
    schema: Arc<Schema>,
    len: usize,

    obj_type_builder: StringBuilder,
    obj_hash_builder: BinaryBuilder,
}

impl ObjectArrayArrowWriter {
    pub fn new(len: usize) -> Self {
        let mut obj_type_builder = StringBuilder::with_capacity(len, len * 40);
        let mut obj_hash_builder = BinaryBuilder::with_capacity(len, len * 40);

        let schema = Schema::new(vec![
            Field::new("obj_type", DataType::Utf8, false),
            Field::new("obj_hash", DataType::Binary, false),
        ]);

        Self {
            schema: Arc::new(schema),
            len,
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

    async fn flush(&mut self, file: &Path) -> NdnResult<()> {
        let obj_type_array: ArrayRef = Arc::new(self.obj_type_builder.finish());
        let obj_hash_array: ArrayRef = Arc::new(self.obj_hash_builder.finish());

        let batch = RecordBatch::try_new(
            self.schema.clone(),
            vec![obj_type_array, obj_hash_array],
        ).map_err(|e| {
            let msg = format!("Failed to create record batch: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let file = File::create(&file).map_err(|e| {
            let msg = format!("Failed to create file: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let mut writer = FileWriter::try_new(file, &self.schema).map_err(|e| {
            let msg = format!("Failed to create file writer: {}", e);
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

        Ok(())
    }
}


pub struct ObjectArrayArrowReader {
    schema: Arc<Schema>,
    batch: RecordBatch,
    len: usize,
}

impl ObjectArrayArrowReader {
    pub fn new(len: usize, schema: Arc<Schema>, batch: RecordBatch) -> Self {
        Self {
            schema,
            batch,
            len,
        }
    }

    async fn open(file: &Path) -> NdnResult<Self> {
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

        let ret = Self::new(len, schema, batch);
        Ok(ret)
    }

    async fn get(&self, index: usize) -> NdnResult<Option<ObjId>> {
        if index >= self.len {
            let msg = format!("Index out of bounds: {} >= {}", index, self.len);
            error!("{}", msg);
            return Err(NdnError::OffsetTooLarge(msg));
        }

        let obj_type_array = self.batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        let obj_hash_array = self.batch.column(1).as_any().downcast_ref::<BinaryArray>().unwrap();

        let obj_type = obj_type_array.value(index).to_string();
        let obj_hash = obj_hash_array.value(index);

        Ok(Some(ObjId::new_by_raw(obj_type, obj_hash.to_vec())))
    }

    async fn len(&self) -> NdnResult<usize> {
        Ok(self.len)
    }
}