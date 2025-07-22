use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::Local;
use scraper::{Html, Selector};
use tokio::{self};

const _6MINUTES_ENGLISH: &'static str =
    "https://www.bbc.co.uk/programmes/p02pc9tn/episodes/downloads";
const _6MINUTES_VOCABULARY: &'static str =
    "https://www.bbc.co.uk/programmes/p02pc9xz/episodes/downloads";
const _6MINUTES_GRAMMAR: &'static str =
    "https://www.bbc.co.uk/programmes/p02pc9wq/episodes/downloads";

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
        fs::create_dir_all(&download_folder)?; // Use create_dir_all instead of create_dir

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
            .await // Add await here
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
                    let filename = self.extract_filename(element)?;
                    download_tasks.push((href.to_string(), filename));
                }
            }
        }

        let total = download_tasks.len();
        if total == 0 {
            println!("No new {} episodes found", self.config.name);
            return Ok(());
        }

        println!("Found {} new episodes, downloading...", total);

        // Use tokio tasks instead of threads for concurrent downloads
        let mut handles = Vec::new();
        let completed = Arc::new(Mutex::new(0));
        let failed = Arc::new(Mutex::new(0));

        for (url, filename) in download_tasks {
            let filepath = self.config.download_folder.join(&filename);
            let index_file = self.index_file.clone();
            let completed = Arc::clone(&completed);
            let failed = Arc::clone(&failed);

            let handle = tokio::spawn(async move {
                match Self::download_file(&url, &filepath).await {
                    Ok(_) => {
                        if let Err(e) = Self::record_download(&index_file, &url) {
                            eprintln!("Failed to record download: {}", e);
                        }
                        let mut comp = completed.lock().unwrap();
                        *comp += 1;
                        println!("Downloaded {}/{}: {}", *comp, total, filename);
                    }
                    Err(e) => {
                        eprintln!("Failed to download {}: {}", filename, e);
                        *failed.lock().unwrap() += 1;
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all downloads to complete
        for handle in handles {
            handle.await.unwrap();
        }

        let completed = *completed.lock().unwrap();
        let failed = *failed.lock().unwrap();

        println!(
            "Download completed: {} succeeded, {} failed",
            completed, failed
        );

        Ok(())
    }

    fn is_already_downloaded(&self, url: &str) -> io::Result<bool> {
        let content = fs::read_to_string(&self.index_file)?;
        Ok(content.contains(url))
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
        let response = reqwest::get(url)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let bytes = response
            .bytes()
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Write bytes to file synchronously
        fs::write(path, bytes)?;

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

// Make main async
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

    // Process podcasts concurrently using tokio tasks
    let mut handles = Vec::new();

    for config in podcasts {
        let handle = tokio::spawn(async move {
            match PodcastDownloader::new(
                &config.name,
                &config.url,
                &config.download_folder.to_str().unwrap(),
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
        });

        handles.push(handle);
    }

    // Wait for all podcast downloads to complete
    for handle in handles {
        handle.await.unwrap();
    }

    println!("All podcast downloads completed!");
    Ok(())
}
