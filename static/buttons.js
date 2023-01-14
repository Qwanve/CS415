window.onload = function() {
    let deal_button = document.getElementById("deal");
    deal_button.onclick = function() {
        let xhr = new XMLHttpRequest();
        let url = "/deal";
        xhr.open("POST", url, true);
        // xhr.setRequestHeader("Content-Type", "application/json");
        xhr.onreadystatechange = function () {
            if (xhr.readyState === 4 && xhr.status === 200) {
                var json = JSON.parse(xhr.responseText);
                console.log(json);
                if (json === null) {
                    deal_button.disabled = true;
                } else {
                    addCard(json);
                }
            } else if (xhr.readyState === 4 && xhr.status !== 200) {
                console.error("Server error. Failed to deal next card");
                deal_button.disabled = true;
            }  
        };
        xhr.send();
    }

    let shuffle_button = document.getElementById("shuffle");
    shuffle_button.onclick = function() {
        let xhr = new XMLHttpRequest();
        let url = "/shuffle";
        xhr.open("POST", url, true);
        // xhr.setRequestHeader("Content-Type", "application/json");
        xhr.onreadystatechange = function() {
            if (xhr.readyState === 4 && xhr.status === 200) {
                console.log("Shuffled");
                clearCards();
                deal_button.disabled = false;
            }
        };
        xhr.send();
    }
}


function addCard(card) {
    let name = "" + card.rank + card.suit;
    let url = "/static/cards/" + name + ".svg";
    let img = document.createElement('img');
    img.src = url;
    img.alt = name;
    let div = document.getElementById("cards");
    div.appendChild(img);
}

function clearCards() {
    let div = document.getElementById("cards");
    while (div.firstChild !== null) {
        div.removeChild(div.firstChild);
    }
}
