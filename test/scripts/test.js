import "node-fetch"
import { writeFile } from "fs/promises"

async function downloadFile(url, outputPath) {
    let res = fetch(url)
    console.log(await res)
    
    return res.then(x => x.arrayBuffer())
      .then(x => writeFile(outputPath, Buffer.from(x)));
}

console.log(await(downloadFile(
    'https://3000-pickaxe828-ttsservice-3ax3kla6sfq.ws-us114.gitpod.io/tts?text="pɪˈkɑːn"&lang=Salli&mode=Polly&preferred_format="audio/ogg"', 
    "./test.ogg"
)))