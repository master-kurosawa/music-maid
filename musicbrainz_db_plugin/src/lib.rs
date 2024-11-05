use search_proto::search_client::SearchClient;
use search_proto::SearchReleaseRequest;
use tonic::transport::Channel;

pub mod search_proto {
    tonic::include_proto!("search");
}

pub async fn create_client(
) -> Result<SearchClient<Channel>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(SearchClient::connect("http://[::1]:50051").await?)
}

pub async fn search(client: &mut SearchClient<Channel>, query: String) {
    let request = tonic::Request::new(SearchReleaseRequest {
        name: "kuroi uta".to_string(),
    });
    let x = client.search_release(request).await;
    println!("{:?}", x)
}
