code = '''
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StitchRequest {
    pub files: Vec<String>,
    pub output_path: String,
}

pub async fn stitch_images(Json(payload): Json<StitchRequest>) -> Result<Json<Value>, ApiError> {
    let result = tokio::task::spawn_blocking(move || {
        let paths: Vec<PathBuf> = payload.files.iter().map(PathBuf::from).collect();
        let options = snow_stitch_images::StitchOptions::default();
        let res = snow_stitch_images::stitch_files(&paths, options).map_err(|e| e.to_string())?;
        
        if let Some(frame) = res.image() {
            frame.encode(std::path::Path::new(&payload.output_path)).map_err(|e| e.to_string())?;
            Ok(payload.output_path)
        } else {
            Err("Failed to extract stitched image".to_string())
        }
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    match result {
        Ok(path) => Ok(Json(json!({ "status": "success", "outputPath": path }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e })))),
    }
}
'''
with open('D:/mowen/deepcloud/super-agent/hub-rs/src/routes/vision.rs', 'a', encoding='utf-8') as f:
    f.write(code)
