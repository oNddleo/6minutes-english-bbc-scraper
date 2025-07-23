use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use chrono::Local;
use futures::future::join_all;
use scraper::{Html, Selector};

const _6MINUTES_ENGLISH: &str = "https://www.bbc.co.uk/programmes/p02pc9tn/episodes/downloads";
const _6MINUTES_VOCABULARY: &str = "https://www.bbc.co.uk/programmes/p02pc9xz/episodes/downloads";
const _6MINUTES_GRAMMAR: &str = "https://www.bbc.co.uk/programmes/p02pc9wq/episodes/downloads";

#[derive(Debug, Clone)]
pub struct PodcastConfig {
    pub name: String,
    pub url: String,
    pub download_folder: PathBuf,
}

pub struct PodcastDownloader {
    config: PodcastConfig,
    index_file: PathBuf,
}

impl PodcastDownloader {
    fn new(name: &str, url: &str, folder: &str) -> io::Result<Self> {
        let download_folder = Path::new(folder).to_path_buf();
        fs::create_dir_all(&download_folder)?;

        let index_file = download_folder.join(".podcast_index");

        if !index_file.exists() {
            let mut file = File::create(&index_file)?;
            writeln!(file, "Generate Podcast Downloader\n{}", "-".repeat(40))?;
        }
        Ok(Self {
            config: PodcastConfig {
                name: name.to_string(),
                url: url.to_string(),
                download_folder,
            },
            index_file,
        })
    }

    pub async fn download_episodes(&self) -> io::Result<()> {
        println!("Checking for new {} episodes...", self.config.name);

        let response = reqwest::get(&self.config.url)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let html = response
            .text()
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let document = Html::parse_document(&html);
        let selector = Selector::parse("a[href$=\".mp3\"]")
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Invalid selector"))?;

        // Collect all download links first
        let mut download_tasks = Vec::new();

        for element in document.select(&selector) {
            if let Some(href) = element.value().attr("href") {
                if !href.contains("audio-nondrm-download-low")
                    && !self.is_already_downloaded(href)?
                {
                    if let Ok(filename) = self.extract_filename(element) {
                        download_tasks.push((href.to_string(), filename));
                    }
                }
            }
        }

        let total = download_tasks.len();
        if total == 0 {
            println!("No new {} episodes found", self.config.name);
            return Ok(());
        }

        println!("Found {} new episodes, downloading...", total);

        // Use futures::future::join_all for concurrent downloads without spawning
        let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
        let completed = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicUsize::new(0));

        let download_futures = download_tasks
            .into_iter()
            .map(|(url, filename)| {
                let semaphore = Arc::clone(&semaphore);
                let completed = Arc::clone(&completed);
                let failed = Arc::clone(&failed);
                let download_folder = self.config.download_folder.clone();
                let index_file = self.index_file.clone();

                async move {
                    let _permit = semaphore.acquire().await.unwrap();

                    let filepath = download_folder.join(&filename);

                    match Self::download_file(&url, &filepath).await {
                        Ok(_) => {
                            if let Err(e) = Self::record_download(&index_file, &url) {
                                eprintln!("Failed to record download: {}", e);
                            }
                            let comp_count = completed.fetch_add(1, Ordering::SeqCst) + 1;
                            println!("Downloaded {}/{}: {}", comp_count, total, filename);
                        }
                        Err(e) => {
                            eprintln!("Failed to download {}: {}", filename, e);
                            failed.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            })
            .collect::<Vec<_>>();

        // Wait for all downloads to complete
        join_all(download_futures).await;

        let completed = completed.load(Ordering::SeqCst);
        let failed = failed.load(Ordering::SeqCst);

        println!(
            "Download: {} completed, {} failed",
            completed, failed
        );

        Ok(())
    }

    fn is_already_downloaded(&self, url: &str) -> io::Result<bool> {
        match fs::read_to_string(&self.index_file) {
            Ok(content) => Ok(content.contains(url)),
            Err(_) => Ok(false),
        }
    }

    fn extract_filename(&self, element: scraper::ElementRef) -> io::Result<String> {
        if let Some(download_attr) = element.value().attr("download") {
            let clean_name = download_attr.replace(" ", "_");
            Ok(clean_name
                .split(",")
                .nth(1)
                .unwrap_or(&clean_name)
                .to_string())
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "No download attribute found",
            ))
        }
    }

    async fn download_file(url: &str, path: &Path) -> io::Result<()> {
        let response = reqwest::get(format!("https:{}", url))
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let bytes = response
            .bytes()
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    fn record_download(index_file: &Path, url: &str) -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(index_file)?;
        writeln!(file, "{} {}", Local::now().format("%Y%m%d%H%M%S"), url)?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    println!("BBC Scraper\n");

    let podcasts = vec![
        PodcastConfig {
            name: "6MinuteEnglish".to_string(),
            url: "https://www.bbc.co.uk/programmes/p02pc9tn/episodes/downloads".to_string(),
            download_folder: PathBuf::from("./podcasts/6min_english"),
        },
        PodcastConfig {
            name: "6 Minute Vocabulary".to_string(),
            url: "https://www.bbc.co.uk/programmes/p02pc9xz/episodes/downloads".to_string(),
            download_folder: PathBuf::from("./podcasts/6min_vocabulary"),
        },
        PodcastConfig {
            name: "6 Minute Grammar".to_string(),
            url: "https://www.bbc.co.uk/programmes/p02pc9wq/episodes/downloads".to_string(),
            download_folder: PathBuf::from("./podcasts/6min_grammar"),
        },
    ];

    // Process podcasts sequentially to avoid Send issues
    for config in podcasts {
        match PodcastDownloader::new(
            &config.name,
            &config.url,
            config.download_folder.to_str().unwrap(),
        ) {
            Ok(downloader) => {
                println!("Starting downloader for {}...", config.name);

                if let Err(e) = downloader.download_episodes().await {
                    eprintln!("Error downloading {}: {}", config.name, e);
                }

                println!("Finished {}", config.name);
            }
            Err(e) => {
                eprintln!("Failed to initialize {} downloader: {}", config.name, e);
            }
        }
    }

    println!("All podcast downloads completed!");
    Ok(())
}
