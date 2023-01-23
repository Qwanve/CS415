window.onload = function() {
    let ws = new WebSocket("ws://localhost:3000/ws");
    let next_button = document.getElementById("next");
    let clear_button = document.getElementById("clear");

    ws.onopen = function() {
        next_button.disabled = false;
        clear_button.disabled = false;
    }
    
    next_button.onclick = function() {
        ws.send(JSON.stringify("Next"));
    }

    clear_button.onclick = function() {
        ws.send(JSON.stringify("Clear"));
    }

    ws.onmessage = (event) => {
        let msg = JSON.parse(event.data);
        console.log(msg);
        if (Number.isInteger(msg)) {
            addNumber(msg);
        } else if (msg == "Clear") {
            clearNumbers();
        }
    }
}


function addNumber(num) {
    let img = document.createElement('li');
    img.innerHTML = num;
    let ul = document.getElementById("numbers");
    ul.appendChild(img);
}

function clearNumbers() {
    let div = document.getElementById("numbers");
    while (div.firstChild !== null) {
        div.removeChild(div.firstChild);
    }
}
