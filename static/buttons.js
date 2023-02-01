window.onload = function() {
    let ws = null;
    let join_button = document.getElementById("join");
    let click_button = document.getElementById("clickme");

    join_button.onclick = function() {
        ws = new WebSocket("ws://localhost:3000/ws");
        ws.onopen = function() {
            join_button.disabled = true;
            join_button.hidden = true;
            click_button.hidden = false;
        }
        ws.onmessage = (event) => {
            let msg = JSON.parse(event.data);
            console.log(msg);
            if (msg === "YourTurn") {
                click_button.disabled = false;
            } else {
                console.error("Unknown message from server");
            }
        }
    }

    click_button.onclick = function() {
        ws.send(JSON.stringify("Click"));
        click_button.disabled = true;
    }
}
