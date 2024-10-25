use axum::{
    async_trait, extract::{FromRequestParts, Multipart}, http::{request::Parts, StatusCode}, routing::{get, post}, Extension, Json, Router
};
use std::collections::HashMap;
use tower_http::cors::CorsLayer;
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};

use aws_sdk_s3 as s3;

use s3::Client;

#[tokio::main]
async fn main() {
    // configuration for logging    
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "aws_image_upload=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();
    // configure your cors setting
    let cors_layer = CorsLayer::permissive();

    // the aws credentials from environment
    let aws_configuration = aws_config::load_from_env().await;
    //create aws s3 client
    let aws_s3_client = s3::Client::new(&aws_configuration);

    let app = Router::new()
        // route for testing if api is running correctly
        .route("/", get(|| async move { "welcome to Image upload api" }))
        //route for uploading image or any file
        .route("/upload", post(upload_image))
        // set your cors config
        .layer(cors_layer)
        // pass the aws s3 client to route handler
        .layer(Extension(aws_s3_client));
    let addr = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(addr, app)
        .await
        .expect("Failed to start server");

}

struct ApiKey(String);


#[async_trait]
impl<B> FromRequestParts<B> for ApiKey
where
    B: Send + Sync,
{
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(parts: &mut Parts, _state: &B) -> Result<Self, Self::Rejection> {
        // Extract the x-api-key header
        let api_key_header = parts
            .headers
            .get("x-api-key")
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "API key is missing"})),
                )
            })?;

        // Convert the header value to a string
        let api_key_str = api_key_header
            .to_str()
            .map_err(|_| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "Invalid API key format"})),
                )
            })?;

        // Define the hardcoded API key
        let expected_api_key = std::env::var("API_KEY").unwrap_or_else(|_| "api-key-vg-11".to_owned());

        // Verify the API key
        if api_key_str == expected_api_key {
            Ok(ApiKey(api_key_str.to_string()))
        } else {
            Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Unauthorized"})),
            ))
        }
    }
}

// handler to upload image or file
async fn upload_image(
    ApiKey(_): ApiKey,
    Extension(s3_client): Extension<Client>,
    mut files: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let bucket = std::env::var("AWS_S3_BUCKET").unwrap_or_else(|_| "my-bucket-name".to_owned());
    // if you have a public url for your bucket, place it as ENV variable BUCKET_URL
    let bucket_url = std::env::var("BUCKET_URL").unwrap_or_else(|_| bucket.to_owned());
    // if a sub folder path is set in environment then get that else set a default sub path
    let sub_path = std::env::var("BUCKET_SUB_PATH").unwrap_or_else(|_| "uploaded_images".to_owned());
    // we are going to store the respose in HashMap as filename: url => key: value
    let mut res = HashMap::new();
    while let Some(file) = files.next_field().await.unwrap() {
        // this is the name which is sent in formdata from frontend or whoever called the api, i am
        // using it as category, we can get the filename from file data
        let category = file.name().unwrap().to_string();
        // name of the file with extention
        let name = file.file_name().unwrap().to_string();
        // file data
        let data = file.bytes().await.unwrap();
        // the request can control where to save the image on AWS S3
        let request_path = if category.trim().is_empty() {"".to_owned()} else {format!("{}/", &category.trim_end_matches('/'))};
        // the path of file to store on aws s3 with file name and extention
        // timestamp_category_filename => 14-12-2022_01:01:01_customer_somecustomer.jpg
        let key = format!(
            "{}/images/{}{}_{}_{}",
            sub_path,
            &request_path,
            chrono::Utc::now().format("%d-%m-%Y_%H:%M:%S"),
            &category,
            &name
        );

        // send Putobject request to aws s3
        let _resp = s3_client
            .put_object()
            .bucket(&bucket)
            .key(&key)
            .body(data.into())
            .send()
            .await
            .map_err(|err| {
                println!("Error occurred during image upload: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"err": "an error occured during image upload"})),
                )
            })?;
        dbg!(_resp);
        let url = format!("{}/{}", bucket_url, key);
        println!("Uploaded file URL: {}", url);
        res.insert(
            // concatinating name and category so even if the filenames are same it will not
            // conflict
            format!("{}_{}", &name, &category),
            url,
        );
    }
    // send the urls in response
    Ok(Json(serde_json::json!(res)))
}