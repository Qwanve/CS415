window.onload = function() {
  let end_turn_button = document.getElementById("endturn");
  let deal_button = document.getElementById("deal");
  let split_button = document.getElementById("split");
  let ws = new WebSocket("ws://localhost:3000" + window.location.pathname + "/ws");
  ws.onopen = function() {
    console.log("Connection Made");
    end_turn_button.onclick = function() {
      ws.send(JSON.stringify("EndTurn"));
      end_turn_button.disabled = true;
      deal_button.disabled = true;
    }
    deal_button.onclick = function() {
      ws.send(JSON.stringify("Deal"));
    }
    split_button.onclick = function() {
      ws.send(JSON.stringify("Split"));
      split_button.hidden = true;
      split_button.disabled = true;
    }
  }
  let player_count = 0;
  ws.onmessage = function(event) {
    let msg = JSON.parse(event.data);
    console.log(msg);
    if (msg === "EndTurn") {

      deal_button.disabled = true;

    } else if (msg === "NewHost") {

      let start_button = document.getElementById("start");
      start_button.removeAttribute("hidden");
      start_button.disabled = false;
      start_button.onclick = function() {
        ws.send(JSON.stringify("GameStart"));
        start_button.disabled = true;
        start_button.hidden = true;
      }

    } else if (msg === "RequestBet") {

      let bet_button = document.getElementById("bet");
      let bet_value = document.getElementById("bet_label");
      let bet_slider = document.getElementById("bet_amount");
      bet_slider.onchange = function() {
        bet_value.innerHTML = bet_slider.value;
      }
      bet_value.removeAttribute("hidden");
      bet_button.removeAttribute("hidden");
      bet_slider.removeAttribute("hidden");
      bet_button.onclick = function() {
        let msg = {"Bet":{"amount":Number(bet_slider.value)}};
        ws.send(JSON.stringify(msg));
        bet_button.hidden = "true";
        bet_slider.hidden = "true";
        bet_value.hidden = "true";
        bet_slider.max -= bet_slider.value;
      }

    } else if (msg.hasOwnProperty('YourTurn')) {

      end_turn_button.disabled = false;
      deal_button.disabled = false;
      if (msg.YourTurn.can_split) {
        console.log("You can split");
        split_button.removeAttribute("hidden");
        split_button.disabled = false;
      }

    } else if (msg.hasOwnProperty('PlayerSplit')) {

      let id = "player" + msg.PlayerSplit.player;
      document.getElementById(id + ".1").removeAttribute("hidden");
      document.getElementById(id + ".1").appendChild(document.getElementById(id).firstChild);

    } else if (msg.hasOwnProperty('EndGame')) {

      let res = msg.EndGame.result;
      if (res === "Lose") {
        alert("Game has ended. You lost.");
      } else if (res === "Win") {
        alert("Game has ended. You won.");
      } else if (res === "Blackjack") {
        alert("Game has ended. You won (Blackjack).");
      } else if (res === "Push") {
        alert("Game has ended. You tied.");
      } else {
        alert("Game has ended.");
      }
      deal_button.disabled = true;
      end_turn_button.disabled = true;
      let dealer_hand = msg.EndGame.dealer_hand;
      let dealer = document.getElementById("dealer");
      let imgs = Array.from(dealer.children);
      for (idx in dealer_hand) {
        let card = dealer_hand[idx]
        imgs[idx].src = "/static/cards/" + card.rank + card.suit + ".svg";
      }
      
      ws.close();
      setTimeout(() => location.href = "/", 5000);
      
    } else if (msg.hasOwnProperty('PlayerJoin')) {

      player_count = msg.PlayerJoin.player;
      for (let i = 0; i < msg.PlayerJoin.player; i++) {
        let player = document.getElementById("player" + i);
        player.removeAttribute("hidden");
      }

    } else if (msg.hasOwnProperty('PlayerLeave')) {

      let player_leaving = document.getElementById("player" + msg.PlayerLeave.player);
      player_leaving.innerHTML = "";
      player_leaving.hidden = "true";
      let player_leaving_split = document.getElementById("player" + msg.PlayerLeave.player + ".1");
      player_leaving_split.innerHTML = "";
      player_leaving_split.hidden = "true";
      for(let i = msg.PlayerLeave.player; i < player_count; i++) {
        let oldParent = document.getElementById("player" + (i + 1));
        let newParent = document.getElementById("player" + i);
        while(oldParent.hasChildNodes()) {
          console.log("moving card from " + oldParent.id + " to " + newParent.id);
          newParent.append(oldParent.firstChild);
        }
        let oldParentSplit = document.getElementById("player" + (i + 1) + ".1");
        let newParentSplit = document.getElementById("player" + i + ".1");
        if (!oldParentSplit.hasAttribute("hidden")) {
          newParentSplit.removeAttribute("hidden");
          while(oldParentSplit.hasChildNodes()) {
            console.log("moving card from " + oldParent.id + ".1 to " + newParent.id);
            newParentSplit.append(oldParentSplit.firstChild);
          }
          oldParentSplit.setAttribute("hidden", "true");
        }
      }
      console.log("player_count:" + player_count);
      let player = document.getElementById("player" + (player_count - 1));
      player.setAttribute("hidden", "true");

      let player_split = document.getElementById("player" + (player_count - 1) + ".1");
      player_split.setAttribute("hidden", "true");
      player_count--;

    } else if (msg.hasOwnProperty('Dealt')) {

      let card = msg.Dealt.card;
      let img = document.createElement("img");
      if (card !== null) {
        card = "" + card.rank + " of " + card.suit;
        console.log("Player " + msg.Dealt.hand + " has recieved the card " + card);
        img.src = "/static/cards/" + msg.Dealt.card.rank + msg.Dealt.card.suit + ".svg";
        
      } else {
        console.log("Player " + msg.Dealt.hand + " has recieved a card");
        img.src = "/static/cards/back.svg";
      }
      img.style = "width: 20%;";
      let id = "player" + msg.Dealt.hand;
      if (msg.Dealt.second_hand) {
        id += ".1";
      }
      document.getElementById(id).appendChild(img);

    } else if (msg.hasOwnProperty("DealDealer")) {

      let card = msg.DealDealer.card;
      let img = document.createElement("img");
      if (card !== null) {
        card = "" + card.rank + " of " + card.suit;
        console.log("Dealer has recieved the card " + card);
        img.src = "/static/cards/" + msg.DealDealer.card.rank + msg.DealDealer.card.suit + ".svg";
        
      } else {
        console.log("Dealer has recieved a card");
        img.src = "/static/cards/back.svg";
      }
      img.style = "width: 20%;";
      document.getElementById("dealer").appendChild(img);

    }
  }
}
