/*
1) get db instance by appid+dbname + init_schema(with version)
2）db instance is a connection string
3) user use the lib the like to connect to the db
*/ 

//owner_user_id is optional, if not provided, appid is service_id
// return a connection string like
// postgres://app:secret@127.0.0.1:5432/mydb
// sqlite:///$appdata/data.db
pub async fn get_rdb_instance(appid: &str, owner_user_id: Option<String>, dbname: &str) {
    unimplemented!()
}